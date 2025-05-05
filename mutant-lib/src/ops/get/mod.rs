use crate::error::Error;
use crate::events::{GetCallback, GetEvent};
use crate::index::{master_index::MasterIndex, PadInfo};
use crate::internal_events::invoke_get_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError, BATCH_SIZE, NB_CLIENTS};
use crate::ops::worker::{AsyncTask, PoolError, WorkerPool};
use async_channel::bounded;
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error};
use std::sync::atomic::Ordering;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, RwLock};

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
    drop(client_guard);

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

    let pads_to_fetch = pads; // Use the vector directly

    fetch_pads_data(network, pads_to_fetch, is_public, callback).await
}

// Context for the GET AsyncTask
#[derive(Clone)] // Required by WorkerPool context
struct GetContext {
    network: Arc<Network>,
    public: bool,
    get_callback: Option<GetCallback>,
    // These are needed by the pool but less critical for GET's logic itself
    completion_notifier: Arc<Notify>,
    total_items: Arc<std::sync::atomic::AtomicUsize>,
    fetched_items_counter: Arc<std::sync::atomic::AtomicUsize>,
}

// Task processor for GET operations.
#[derive(Clone)] // Required by WorkerPool
struct GetTaskProcessor;

// We need to use a different approach since Object<autonomi::Client> doesn't implement Clone
// Let's use Arc<Mutex<Object<autonomi::Client>>> as our client type
#[async_trait]
impl AsyncTask<PadInfo, GetContext, Object<crate::network::client::ClientManager>, Vec<u8>, Error>
    for GetTaskProcessor
{
    type ItemId = usize; // Use chunk index for ordering

    async fn process(
        &self,
        _worker_id: usize,
        context: Arc<GetContext>,
        client: &Object<crate::network::client::ClientManager>, // Changed client type and removed Arc/Mutex
        pad: PadInfo,
    ) -> Result<(Self::ItemId, Vec<u8>), (Error, PadInfo)> {
        let mut retries_left = PAD_RECYCLING_RETRIES; // Use same retry count logic
        let owned_key;
        let secret_key_ref = if context.public {
            None
        } else {
            owned_key = pad.secret_key();
            Some(&owned_key)
        };

        loop {
            // Removed client mutex locking
            // Use the provided client directly
            match context
                .network
                .get(client, &pad.address, secret_key_ref) // Pass client directly
                .await
            {
                Ok(pad_result) => {
                    // Pad size check
                    let counter_match = pad_result.counter == pad.last_known_counter;
                    let size_match = pad_result.data.len() == pad.size;
                    let checksum_match = pad.checksum == PadInfo::checksum(&pad_result.data);
                    if !counter_match || !size_match || !checksum_match {
                        error!(
                            "Pad mismatch for pad {}: Match: counter: {}, size: {}, checksum: {}. Retrying...",
                            pad.address, counter_match, size_match, checksum_match
                        );
                        continue;
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
                    retries_left -= 1;
                    if retries_left == 0 {
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
    // Create worker-specific channels and a global queue channel
    let mut worker_txs = Vec::with_capacity(*NB_CLIENTS);
    let mut worker_rxs = Vec::with_capacity(*NB_CLIENTS);
    for _ in 0..*NB_CLIENTS {
        // Bounded channel for each worker's initial tasks
        let (tx, rx) =
            bounded::<PadInfo>(total_pads_to_fetch.saturating_add(1) / *NB_CLIENTS + *BATCH_SIZE);
        worker_txs.push(tx);
        worker_rxs.push(rx);
    }
    // Global queue - might not be strictly necessary for GET if no recycling
    let (global_tx, global_rx) =
        bounded::<PadInfo>(total_pads_to_fetch + *NB_CLIENTS * *BATCH_SIZE); // Generous buffer

    // Create context for the worker pool
    let get_context = Arc::new(GetContext {
        network: network.clone(),
        public,
        get_callback: get_callback.clone(),
        completion_notifier: Arc::new(Notify::new()),
        total_items: Arc::new(std::sync::atomic::AtomicUsize::new(total_pads_to_fetch)),
        fetched_items_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });

    // Task to send pads to the pool
    let send_pads_task = {
        let pads_clone = pads.clone();
        // Clone worker transmitters
        let worker_txs_clone = worker_txs.clone();
        let global_tx_clone = global_tx.clone(); // Need to close this later
        let completion_notifier_clone = get_context.completion_notifier.clone();
        let fetched_items_counter_clone = get_context.fetched_items_counter.clone();
        let total_items_clone = get_context.total_items.clone();

        tokio::spawn(async move {
            let mut worker_index = 0;
            let total_pads = pads_clone.len();

            debug!(
                "Distributing {} pads to {} workers in round-robin fashion",
                total_pads, *NB_CLIENTS
            );

            for pad in pads_clone {
                // Send round-robin to worker channels
                let target_tx = &worker_txs_clone[worker_index % *NB_CLIENTS];
                if target_tx.send(pad).await.is_err() {
                    break;
                }
                worker_index += 1;
            }

            // Wait for all pads to be fetched before closing channels
            // This ensures that all task processors stay alive until all work is done
            loop {
                let fetched_count = fetched_items_counter_clone.load(Ordering::SeqCst);
                let total_count = total_items_clone.load(Ordering::SeqCst);

                if fetched_count >= total_count {
                    debug!(
                        "All pads fetched ({}/{}), closing channels",
                        fetched_count, total_count
                    );
                    break;
                }

                // Wait for completion notification or check again after a delay
                tokio::select! {
                    _ = completion_notifier_clone.notified() => {
                        debug!("Received completion notification, closing channels");
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        // Continue checking
                    }
                }
            }

            // Only close channels after all work is done
            for tx in worker_txs_clone {
                tx.close();
            }

            // Also close the global transmitter after all worker channels are closed
            global_tx_clone.close();
        })
    };

    // Create clients for each worker - EXACTLY ONE client per worker
    let mut clients = Vec::with_capacity(*NB_CLIENTS);
    for worker_id in 0..*NB_CLIENTS {
        let client = network.get_client(Config::Get).await.map_err(|e| {
            error!("Failed to get client for worker {}: {}", worker_id, e);
            Error::Network(NetworkError::ClientAccessError(format!(
                "Failed to get client for worker {}: {}",
                worker_id, e
            )))
        })?;
        clients.push(Arc::new(client)); // Use Arc<Object> directly, matching put
    }

    // Create and configure the worker pool
    let pool = WorkerPool::new(
        *NB_CLIENTS,
        *BATCH_SIZE,
        get_context.clone(),
        Arc::new(GetTaskProcessor),
        clients,    // Pass Vec<Arc<Object<ClientManager>>>
        worker_rxs, // Pass worker-specific receivers
        global_rx,  // Pass global receiver
        None,       // No retry channel for GET
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
            if fetched_results.len() != total_pads_to_fetch {
                return Err(Error::Internal(
                    "Mismatch between expected and fetched pad count".to_string(),
                ));
            }

            // Sort results by chunk index (ItemId)
            fetched_results.sort_by_key(|(chunk_index, _)| *chunk_index);

            // Collect into Vec first
            let collected_data: Vec<Vec<u8>> =
                fetched_results.into_iter().map(|(_, data)| data).collect();

            // Concatenate data in order
            let final_capacity: usize = collected_data.iter().map(|data| data.len()).sum();
            let mut final_data: Vec<u8> = Vec::with_capacity(final_capacity);
            for pad_data in collected_data {
                final_data.extend(pad_data);
            }

            // Send completion event after successful processing
            invoke_get_callback(&get_callback, GetEvent::Complete)
                .await
                .unwrap();

            Ok(final_data)
        }
        Err(pool_error) => {
            // Convert PoolError to crate::Error
            let final_error = match pool_error {
                PoolError::TaskError(task_err) => task_err,
                PoolError::JoinError(join_err) => {
                    Error::Internal(format!("Worker task join error: {:?}", join_err))
                }
                PoolError::PoolSetupError(msg) => {
                    Error::Internal(format!("Pool setup error: {}", msg))
                }
                PoolError::WorkerError(msg) => Error::Internal(format!("Worker error: {}", msg)),
            };
            Err(final_error)
        }
    }
}
