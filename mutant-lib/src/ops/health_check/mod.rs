use crate::error::Error;
use crate::index::{master_index::MasterIndex, PadStatus};
use crate::internal_events::invoke_health_check_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError};
use ant_networking::GetRecordError;
use log::{error, warn};
use mutant_protocol::{HealthCheckCallback, HealthCheckEvent, HealthCheckResult};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::RwLock;

pub(super) async fn health_check(
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    recycle: bool,
    health_check_callback: Option<HealthCheckCallback>,
) -> Result<HealthCheckResult, Error> {
    let pads = index.read().await.get_pads(key_name);
    let nb_recycled = Arc::new(AtomicUsize::new(0));
    let nb_reset = Arc::new(AtomicUsize::new(0));
    let callback = health_check_callback.clone();

    invoke_health_check_callback(
        &callback,
        HealthCheckEvent::Starting {
            total_keys: pads.len(),
        },
    )
    .await
    .unwrap();

    let is_public = index.read().await.is_public(key_name);
    let mut tasks = Vec::new();
    let client_guard = network
        .get_client(Config::Get)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client = Arc::new(client_guard);

    for pad in pads {
        let pad = pad.clone();
        let nb_recycled_clone = nb_recycled.clone();
        let nb_reset_clone = nb_reset.clone();
        let key_name_clone = key_name.to_string(); // Clone key_name for the task
        let network_clone = network.clone();
        let task_callback = callback.clone();
        let index_clone = index.clone();
        let client_clone = client.clone();

        tasks.push(tokio::spawn(async move {
            let secret_key_owned;
            let secret_key_ref = if is_public {
                None
            } else {
                secret_key_owned = pad.secret_key();
                Some(&secret_key_owned)
            };

            if pad.status == PadStatus::Generated {
                return;
            }

            let client_ref = &*client_clone;
            match network_clone
                .get(client_ref, &pad.address, secret_key_ref)
                .await
            {
                Ok(_) => {
                    invoke_health_check_callback(&task_callback, HealthCheckEvent::KeyProcessed)
                        .await
                        .unwrap();
                    return;
                }
                Err(e) => {
                    warn!(
                        "Error getting pad {} during health check: {}",
                        pad.address, e
                    );
                    match e {
                        NetworkError::GetError(GetRecordError::NotEnoughCopies { .. }) => {
                            let mut index_guard = index_clone.write().await;
                            index_guard
                                .update_pad_status(
                                    &key_name_clone, // Use cloned key_name
                                    &pad.address,
                                    PadStatus::Free,
                                    None,
                                )
                                .unwrap();
                            nb_reset_clone.fetch_add(1, Ordering::Relaxed);
                        }

                        _ => {
                            if recycle {
                                let mut index_guard = index_clone.write().await;
                                index_guard
                                    .recycle_errored_pad(&key_name_clone, &pad.address) // Use cloned key_name
                                    .await // Add await here
                                    .unwrap();
                            }
                            nb_recycled_clone.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                    invoke_health_check_callback(&task_callback, HealthCheckEvent::KeyProcessed)
                        .await
                        .unwrap();
                }
            }
        }));
    }

    let results = futures::future::join_all(tasks).await;

    for result in results {
        match result {
            Ok(_) => {}
            Err(e) => {
                error!("Health check task panicked: {:?}", e);
            }
        }
    }

    invoke_health_check_callback(
        &callback,
        HealthCheckEvent::Complete {
            nb_keys_updated: nb_reset.load(Ordering::Relaxed),
        },
    )
    .await
    .unwrap();

    println!(
        "Health check completed. {} pads reset.",
        nb_reset.load(Ordering::Relaxed)
    );

    if nb_recycled.load(Ordering::Relaxed) > 0 {
        if recycle {
            println!(
                "{} pads got errored and have been recycled.",
                nb_recycled.load(Ordering::Relaxed)
            );
        } else {
            println!(
                "{} pads got errored and should be recycled.",
                nb_recycled.load(Ordering::Relaxed)
            );
            println!(
                "You can re-run the health-check command with the --recycle flag to recycle them."
            );
        }
    }

    if nb_reset.load(Ordering::Relaxed) > 0 || (nb_recycled.load(Ordering::Relaxed) > 0 && recycle)
    {
        println!("Please re-run the same put command you used before to resume the upload of the missing pads to the network.");
    }

    Ok(HealthCheckResult {
        nb_keys_reset: nb_reset.load(Ordering::Relaxed),
        nb_keys_recycled: nb_recycled.load(Ordering::Relaxed),
    })
}
