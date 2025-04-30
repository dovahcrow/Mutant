use crate::error::Error;
use crate::events::{PurgeCallback, PurgeEvent};
use crate::index::master_index::MasterIndex;
use crate::internal_events::invoke_purge_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError};
use ant_networking::GetRecordError;
use futures::StreamExt;
use log::{debug, error, warn};
use mutant_protocol::PurgeResult;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::RwLock;

pub(super) async fn purge(
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    aggressive: bool,
    purge_callback: Option<PurgeCallback>,
) -> Result<PurgeResult, Error> {
    let pads = index.read().await.get_pending_pads();
    let nb_verified = Arc::new(AtomicUsize::new(0));
    let nb_failed = Arc::new(AtomicUsize::new(0));
    let callback = purge_callback.clone();

    invoke_purge_callback(
        &callback,
        PurgeEvent::Starting {
            total_count: pads.len(),
        },
    )
    .await
    .unwrap();

    let mut tasks = futures::stream::FuturesOrdered::new();
    let client_guard = network
        .get_client(Config::Get)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client = Arc::new(client_guard);

    for pad in pads {
        let pad = pad.clone();
        let network_clone = network.clone();
        let index_clone = index.clone();
        let nb_verified_clone = nb_verified.clone();
        let nb_failed_clone = nb_failed.clone();
        let task_callback = callback.clone();
        let client_clone = client.clone();

        tasks.push_back(tokio::spawn(async move {
            let client_ref = &*client_clone;
            match network_clone.get(client_ref, &pad.address, None).await {
                Ok(_res) => {
                    debug!("Pad {} verified.", pad.address);
                    if let Ok(mut index_guard) = index_clone.try_write() {
                        index_guard.verified_pending_pad(pad).unwrap();
                    } else {
                        error!("Could not acquire write lock for verifying pad {}", pad.address);
                    }
                    nb_verified_clone.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    if let NetworkError::GetError(GetRecordError::RecordNotFound) = e {
                        debug!("Pad {} discarded.", pad.address);
                        if let Ok(mut index_guard) = index_clone.try_write() {
                            index_guard.discard_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for discarding pad {} (NotFound)", pad.address);
                        }
                    } else if let NetworkError::GetError(GetRecordError::NotEnoughCopies { .. }) = e {
                        debug!("Pad {} verified but not enough copies.", pad.address);
                        if let Ok(mut index_guard) = index_clone.try_write() {
                            index_guard.verified_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for discarding pad {} (NotEnoughCopies)", pad.address);
                        }
                    } else if aggressive {
                        debug!("Pad {} discarded (aggressive).", pad.address);
                        if let Ok(mut index_guard) = index_clone.try_write() {
                            index_guard.discard_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for discarding pad {} (aggressive)", pad.address);
                        }
                    } else {
                        warn!("Pad {} was found but got an error ({}). Leaving in pending until purge is run with aggressive flag", pad.address, e);
                    }
                    nb_failed_clone.fetch_add(1, Ordering::Relaxed);
                }
            }
            invoke_purge_callback(&task_callback, PurgeEvent::PadProcessed).await.unwrap();
        }));
    }

    while let Some(result) = tasks.next().await {
        match result {
            Ok(_) => { /* Task completed successfully (logic handled inside task) */ }
            Err(e) => {
                error!("Purge task panicked: {:?}", e);
            }
        }
    }

    invoke_purge_callback(
        &callback,
        PurgeEvent::Complete {
            verified_count: nb_verified.load(Ordering::Relaxed),
            failed_count: nb_failed.load(Ordering::Relaxed),
        },
    )
    .await
    .unwrap();

    Ok(PurgeResult {
        nb_pads_purged: nb_failed.load(Ordering::Relaxed),
    })
}
