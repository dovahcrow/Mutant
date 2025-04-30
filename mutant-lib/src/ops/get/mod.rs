use crate::error::Error;
use crate::events::{GetCallback, GetEvent};
use crate::index::{master_index::MasterIndex, PadInfo};
use crate::internal_events::invoke_get_callback;
use crate::network::client::{ClientManager, Config};
use crate::network::{Network, NetworkError};
use crate::ops::worker::{AsyncTask, PoolError, WorkerPool};
use async_channel::bounded;
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error, info, warn};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Notify, RwLock};

use super::{
    BATCH_SIZE, DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX, PAD_RECYCLING_RETRIES,
    WORKER_COUNT,
};

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
        DATA_ENCODING_PUBLIC_DATA => {
            invoke_get_callback(&callback, GetEvent::Starting { total_chunks: 1 })
                .await
                .unwrap();
            invoke_get_callback(&callback, GetEvent::PadsFetched)
                .await
                .unwrap();
            invoke_get_callback(&callback, GetEvent::Complete)
                .await
                .unwrap();
            Ok(index_pad_data.data)
        }
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
    let is_public = index.read().await.is_public(name);
    let total_chunks = pads.len();

    invoke_get_callback(&callback, GetEvent::Starting { total_chunks })
        .await
        .unwrap();

    let pads_to_fetch = if is_public && pads.len() > 1 {
        pads[1..].to_vec()
    } else {
        pads
    };

    fetch_pads_data(network, pads_to_fetch, is_public, callback).await
}

// Context for the GET AsyncTask
#[derive(Clone)] // Required by WorkerPool context
struct GetContext {
    network: Arc<Network>,
    public: bool,
    get_callback: Option<GetCallback>,
    client_manager: Arc<Object<ClientManager>>, // Pre-fetched client
    // These are needed by the pool but less critical for GET's logic itself
    completion_notifier: Arc<Notify>,
    total_items: Arc<std::sync::atomic::AtomicUsize>,
    fetched_items_counter: Arc<std::sync::atomic::AtomicUsize>,
}

// Task processor for GET operations.
#[derive(Clone)] // Required by WorkerPool
struct GetTaskProcessor;

#[async_trait]
impl AsyncTask<PadInfo, GetContext, Vec<u8>, Error> for GetTaskProcessor {
    type ItemId = usize; // Use chunk index for ordering

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<GetContext>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, Vec<u8>), (Error, PadInfo)> {
        // Return (chunk_index, data) on success
        let client = &*context.client_manager;
        let mut retries_left = PAD_RECYCLING_RETRIES; // Use same retry count logic
        let owned_key;
        let secret_key_ref = if context.public {
            None
        } else {
            owned_key = pad.secret_key();
            Some(&owned_key)
        };

        loop {
            match context
                .network
                .get(client, &pad.address, secret_key_ref)
                .await
            {
                Ok(pad_result) => {
                    // Pad size check
                    if pad_result.data.len() != pad.size {
                        error!(
                            "Pad size mismatch for pad {}: expected {}, got {}",
                            pad.address,
                            pad.size,
                            pad_result.data.len()
                        );
                        return Err((
                            Error::Internal(format!("Pad size mismatch for pad {}", pad.address)),
                            pad,
                        ));
                    }

                    // Invoke callback for progress
                    invoke_get_callback(&context.get_callback, GetEvent::PadsFetched)
                        .await
                        .map_err(|e| {
                            (
                                Error::Internal(format!("Callback error (PadsFetched): {:?}", e)),
                                pad.clone(),
                            )
                        })?;

                    // Increment counter (informational)
                    context
                        .fetched_items_counter
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    // Return chunk index and fetched data
                    return Ok((pad.chunk_index, pad_result.data));
                }
                Err(e) => {
                    warn!(
                        "Worker {} error getting pad {}: {}. Retries left: {}",
                        worker_id, pad.address, e, retries_left
                    );
                    retries_left -= 1;
                    if retries_left == 0 {
                        error!(
                            "Worker {} failed to get pad {} after multiple retries.",
                            worker_id, pad.address
                        );
                        return Err((Error::Network(e), pad)); // Return error and pad after retries
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await; // Wait before retrying
                }
            }
        }
    }
}

async fn fetch_pads_data(
    network: Arc<Network>,
    pads: Vec<PadInfo>,
    public: bool,
    get_callback: Option<GetCallback>,
) -> Result<Vec<u8>, Error> {
    let total_pads_to_fetch = pads.len();

    if total_pads_to_fetch == 0 {
        invoke_get_callback(&get_callback, GetEvent::Complete)
            .await
            .unwrap();
        return Ok(Vec::new());
    }

    // Create channels for the worker pool
    let (pad_tx, pad_rx) = bounded::<PadInfo>(total_pads_to_fetch + WORKER_COUNT);

    // Pre-fetch client guard
    let client_guard = Arc::new(
        network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?,
    );

    // Create context for the worker pool
    let get_context = Arc::new(GetContext {
        network: network.clone(),
        public,
        get_callback: get_callback.clone(),
        client_manager: client_guard,
        completion_notifier: Arc::new(Notify::new()),
        total_items: Arc::new(std::sync::atomic::AtomicUsize::new(total_pads_to_fetch)),
        fetched_items_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });

    // Task to send pads to the pool
    let send_pads_task = {
        let pads_clone = pads.clone();
        let pad_tx_clone = pad_tx.clone();
        tokio::spawn(async move {
            for pad in pads_clone {
                if pad_tx_clone.send(pad).await.is_err() {
                    error!("Failed to send pad to GET worker channel, receiver closed.");
                    break;
                }
            }
            pad_tx_clone.close();
        })
    };

    // Create and configure the worker pool
    let pool = WorkerPool::new(
        WORKER_COUNT,
        BATCH_SIZE,
        get_context.clone(),
        Arc::new(GetTaskProcessor),
        pad_rx,
        None, // No retry channel for GET
        get_context.completion_notifier.clone(),
        Some(get_context.total_items.clone()),
        Some(get_context.fetched_items_counter.clone()),
    );

    // Run the pool
    let pool_run_result = pool.run().await;

    // Ensure the sending task completes
    send_pads_task
        .await
        .map_err(|e| Error::Internal(format!("Send pads task panicked: {:?}", e)))?;

    // Process the results from the pool
    match pool_run_result {
        Ok(mut fetched_results) => {
            info!(
                "GET worker pool finished. Fetched {} pads.",
                fetched_results.len()
            );

            if fetched_results.len() != total_pads_to_fetch {
                error!(
                    "GET worker pool finished but fetched {} pads, expected {}",
                    fetched_results.len(),
                    total_pads_to_fetch
                );
                return Err(Error::Internal(
                    "Mismatch between expected and fetched pad count".to_string(),
                ));
            }

            // Sort results by chunk index (ItemId)
            fetched_results.sort_by_key(|(chunk_index, _)| *chunk_index);

            // Concatenate data in order
            let final_capacity = fetched_results.iter().map(|(_, data)| data.len()).sum();
            let mut final_data = Vec::with_capacity(final_capacity);
            for (_, pad_data) in fetched_results {
                final_data.extend_from_slice(&pad_data);
            }

            // Send completion event after successful processing
            invoke_get_callback(&get_callback, GetEvent::Complete)
                .await
                .unwrap();

            Ok(final_data)
        }
        Err(pool_error) => {
            error!("GET worker pool failed: {:?}", pool_error);
            // Convert PoolError to crate::Error
            let final_error = match pool_error {
                PoolError::TaskError(task_err) => task_err,
                PoolError::JoinError(join_err) => {
                    Error::Internal(format!("Worker task join error: {:?}", join_err))
                }
                PoolError::PoolSetupError(msg) => {
                    Error::Internal(format!("Pool setup error: {}", msg))
                }
                PoolError::SemaphoreClosed => {
                    Error::Internal("Worker semaphore closed unexpectedly".to_string())
                }
                PoolError::SendError => {
                    Error::Internal("Internal pool error sending item to retry queue".to_string())
                }
            };
            Err(final_error)
        }
    }
}
