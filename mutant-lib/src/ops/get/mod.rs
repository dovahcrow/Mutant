use crate::error::Error;
use crate::events::{GetCallback, GetEvent};
use crate::index::{master_index::MasterIndex, PadInfo};
use crate::internal_events::invoke_get_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError};
use autonomi::ScratchpadAddress;
use log::{debug, error};
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

use super::{DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX, PAD_RECYCLING_RETRIES};

pub(super) async fn get_public(
    network: Arc<Network>,
    address: &ScratchpadAddress,
    get_callback: Option<GetCallback>,
) -> Result<Vec<u8>, Error> {
    let client_guard = network
        .get_client(Config::Get)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client = &*client_guard;
    let index_pad_data = network.get(client, address, None).await?;
    let callback = get_callback.clone();

    match index_pad_data.data_encoding {
        DATA_ENCODING_PUBLIC_INDEX => {
            let index: Vec<PadInfo> = serde_cbor::from_slice(&index_pad_data.data)
                .map_err(|e| Error::Internal(format!("Failed to decode public index: {}", e)))?;

            invoke_get_callback(
                &callback,
                GetEvent::Starting {
                    total_chunks: index.len() + 1,
                },
            )
            .await
            .unwrap();

            invoke_get_callback(&callback, GetEvent::PadsFetched)
                .await
                .unwrap();

            fetch_pads_data(network, index, true, callback).await
        }
        DATA_ENCODING_PUBLIC_DATA => Ok(index_pad_data.data),
        _ => Err(Error::Internal(format!(
            "Unexpected data encoding {} found for public address {}",
            index_pad_data.data_encoding, address
        ))),
    }
}

pub(super) async fn get(
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    get_callback: Option<GetCallback>,
) -> Result<Vec<u8>, Error> {
    if !index.read().await.is_finished(name) {
        return Err(Error::Internal(format!(
            "Key {} upload is not finished, cannot get data",
            name
        )));
    }

    let pads = index.read().await.get_pads(name);

    if pads.is_empty() {
        return Err(Error::Internal(format!("No pads found for key {}", name)));
    }

    let callback = get_callback.clone();

    invoke_get_callback(
        &callback,
        GetEvent::Starting {
            total_chunks: pads.len(),
        },
    )
    .await
    .unwrap();

    let is_public = index.read().await.is_public(name);

    let pads_to_fetch = if is_public && pads.len() > 1 {
        pads[1..].to_vec()
    } else {
        pads
    };

    fetch_pads_data(network, pads_to_fetch, is_public, callback).await
}

async fn fetch_pads_data(
    network: Arc<Network>,
    pads: Vec<PadInfo>,
    public: bool,
    get_callback: Option<GetCallback>,
) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    let mut tasks = Vec::new();
    let client_guard = Arc::new(
        network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?,
    );

    for pad in pads {
        let callback_clone = get_callback.clone();
        let network_clone = network.clone();
        let client_guard_clone = client_guard.clone();

        tasks.push(tokio::spawn(async move {
            let client = &*client_guard_clone;
            let mut retries_left = PAD_RECYCLING_RETRIES;
            let owned_key;
            let secret_key_ref = if public {
                None
            } else {
                owned_key = pad.secret_key();
                Some(&owned_key)
            };

            let pad_data = loop {
                match network_clone
                    .get(client, &pad.address, secret_key_ref)
                    .await
                {
                    Ok(pad_result) => {
                        if pad_result.data.len() != pad.size {
                            return Err(Error::Internal(format!(
                                "Pad size mismatch for pad {}: expected {}, got {}",
                                pad.address,
                                pad.size,
                                pad_result.data.len()
                            )));
                        }

                        invoke_get_callback(&callback_clone, GetEvent::PadsFetched)
                            .await
                            .unwrap();

                        break pad_result.data;
                    }
                    Err(e) => {
                        debug!("Error getting pad {}: {}", pad.address, e);
                        retries_left -= 1;
                        if retries_left == 0 {
                            return Err(Error::Network(e));
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
            };

            Ok(pad_data)
        }));
    }

    let results = futures::future::join_all(tasks).await;

    invoke_get_callback(&get_callback, GetEvent::Complete)
        .await
        .unwrap();

    for result in results {
        match result {
            Ok(Ok(pad_data)) => data.extend_from_slice(&pad_data),
            Ok(Err(e)) => {
                error!("Error fetching pad data during get: {:?}", e);
                return Err(e);
            }
            Err(e) => {
                error!("Task panic during get: {:?}", e);
                return Err(Error::Internal(
                    "Task panic during get operation".to_string(),
                ));
            }
        }
    }
    Ok(data)
}
