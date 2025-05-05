use crate::error::Error;
use crate::events::{GetCallback, GetEvent};
use crate::index::{master_index::MasterIndex, PadInfo};
use crate::internal_events::invoke_get_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError, BATCH_SIZE, NB_CLIENTS};
use crate::ops::worker::{self, AsyncTask, PoolError, WorkerPoolConfig};
use async_channel::bounded;
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error, warn};
use mutant_protocol::GetResult;
use std::sync::atomic::Ordering;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::Instant;

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

            invoke_get_callback(&callback, GetEvent::PadFetched)
                .await
                .unwrap();

            fetch_pads_data(network, index, true, callback).await
        }
        DATA_ENCODING_PUBLIC_DATA => {
            invoke_get_callback(&callback, GetEvent::Starting { total_chunks: 1 })
                .await
                .unwrap();
            invoke_get_callback(&callback, GetEvent::PadFetched)
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
struct GetTaskProcessor {
    context: Arc<GetContext>,
}

impl GetTaskProcessor {
    fn new(context: Arc<GetContext>) -> Self {
        Self { context }
    }
}

// We need to use a different approach since Object<autonomi::Client> doesn't implement Clone
// Let's use Arc<Mutex<Object<autonomi::Client>>> as our client type
#[async_trait]
impl AsyncTask<PadInfo, GetContext, Object<crate::network::client::ClientManager>, Vec<u8>, Error>
    for GetTaskProcessor
{
    type ItemId = usize; // Use chunk index for ordering

    async fn process(
        &self,
        _worker_id: usize, // worker_id not used in GET logic currently
        client: &Object<crate::network::client::ClientManager>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, Vec<u8>), (Error, PadInfo)> {
        let mut retries_left = PAD_RECYCLING_RETRIES;
        let owned_key;
        let secret_key_ref = if self.context.public {
            None
        } else {
            owned_key = pad.secret_key();
            Some(&owned_key)
        };

        loop {
            match self
                .context
                .network
                .get(client, &pad.address, secret_key_ref)
                .await
            {
                Ok(record) => {
                    // GET successful
                    invoke_get_callback(&self.context.get_callback, GetEvent::PadFetched)
                        .await
                        .map_err(|e| (e, pad.clone()))?;

                    self.context
                        .fetched_items_counter
                        .fetch_add(1, Ordering::Relaxed);
                    let current_total = self.context.total_items.load(Ordering::Relaxed);
                    let current_fetched =
                        self.context.fetched_items_counter.load(Ordering::Relaxed);

                    if current_fetched >= current_total {
                        self.context.completion_notifier.notify_one();
                    }

                    return Ok((pad.chunk_index, record.data)); // Return chunk index and data
                }
                Err(e) => {
                    match e {
                        // Specific retryable errors for GET (if any - less likely than PUT)
                        // NetworkError::GetError(GetRecordError::Timeout) => { ... }
                        _ => {
                            // Non-retryable or generic error
                            retries_left -= 1;
                            warn!(
                                "GET failed for pad {} (chunk {}): {}. Retries left: {}",
                                pad.address, pad.chunk_index, e, retries_left
                            );
                            if retries_left <= 0 {
                                return Err((e.into(), pad)); // Return error and original pad
                            }
                        }
                    }
                    // Wait before retrying
                    tokio::time::sleep(Duration::from_secs(1)).await;
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

    // 1. Create Context
    let get_context = Arc::new(GetContext {
        network: network.clone(),
        public,
        get_callback: get_callback.clone(),
        completion_notifier: Arc::new(Notify::new()),
        total_items: Arc::new(std::sync::atomic::AtomicUsize::new(total_pads_to_fetch)),
        fetched_items_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });

    // 2. Create Task Processor
    let task_processor = GetTaskProcessor::new(get_context.clone());

    // 3. Create WorkerPoolConfig
    let config = WorkerPoolConfig {
        network: network.clone(),
        client_config: crate::network::client::Config::Get, // Use crate path for Config
        task_processor,
        enable_recycling: false, // No recycling for GET
        total_items_hint: total_pads_to_fetch,
    };

    // 4. Build WorkerPool and Handles
    let (pool, handles) = match worker::build(config).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to build worker pool for GET: {:?}", e);
            return match e {
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
                _ => Err(Error::Internal(format!("Pool build failed: {:?}", e))),
            };
        }
    };

    // 5. Spawn Distribution Task
    let send_pads_task = {
        let pads_clone = pads; // Clone pads for the task
        let worker_txs = handles.worker_txs;
        let completion_notifier_clone = get_context.completion_notifier.clone();
        let fetched_items_counter_clone = get_context.fetched_items_counter.clone();
        let total_items_clone = get_context.total_items.clone();

        tokio::spawn(async move {
            let mut worker_index = 0;
            let num_workers = worker_txs.len();
            if num_workers == 0 {
                warn!("GET distribution task: No workers to distribute to!");
                return;
            }
            debug!(
                "GET distributing {} pads to {} workers...",
                pads_clone.len(),
                num_workers
            );

            for pad in pads_clone {
                let target_tx = &worker_txs[worker_index % num_workers];
                if target_tx.send(pad).await.is_err() {
                    warn!("GET distribution failed: Worker channel closed unexpectedly.");
                    break;
                }
                worker_index += 1;
            }
            debug!("GET initial distribution finished.");

            // Wait until all items are processed or notified
            loop {
                let fetched_count = fetched_items_counter_clone.load(Ordering::SeqCst);
                let total_count = total_items_clone.load(Ordering::SeqCst);
                if fetched_count >= total_count {
                    debug!(
                        "GET all pads processed ({}/{}), closing worker channels.",
                        fetched_count, total_count
                    );
                    break;
                }
                tokio::select! {
                    _ = completion_notifier_clone.notified() => {
                        debug!("GET completion notified, closing worker channels.");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
            // Drop worker_txs implicitly closes channels
            debug!("GET distribution task finished.");
        })
    };

    // 6. No Recycler Task for GET

    // 7. Run the Worker Pool
    let pool_run_result = pool.run().await;

    // 8. Await Distribution Task
    if let Err(e) = send_pads_task.await {
        error!("GET distribution task panicked: {:?}", e);
        return Err(Error::Internal(format!(
            "Pad distribution task failed: {:?}",
            e
        )));
    }
    debug!("GET distribution task completed.");

    // 9. Process Results
    match pool_run_result {
        Ok(mut fetched_results) => {
            if fetched_results.len() != total_pads_to_fetch {
                // This might happen if some tasks failed and returned Err instead of Ok
                warn!(
                    "GET result count mismatch: expected {}, got {}. Some pads might have failed.",
                    total_pads_to_fetch,
                    fetched_results.len()
                );
                // Consider returning an error or partial data depending on requirements
                return Err(Error::Internal(format!(
                    "GET failed: Fetched {} pads, expected {}",
                    fetched_results.len(),
                    total_pads_to_fetch
                )));
            }

            fetched_results.sort_by_key(|(chunk_index, _)| *chunk_index);
            let collected_data: Vec<Vec<u8>> =
                fetched_results.into_iter().map(|(_, data)| data).collect();
            let final_capacity: usize = collected_data.iter().map(|data| data.len()).sum();
            let mut final_data: Vec<u8> = Vec::with_capacity(final_capacity);
            for pad_data in collected_data {
                final_data.extend(pad_data);
            }

            invoke_get_callback(&get_callback, GetEvent::Complete)
                .await
                .unwrap();
            Ok(final_data)
        }
        Err(pool_error) => {
            error!("GET worker pool failed: {:?}", pool_error);
            match pool_error {
                PoolError::TaskError(task_err) => Err(task_err),
                PoolError::JoinError(join_err) => Err(Error::Internal(format!(
                    "Worker task join error: {:?}",
                    join_err
                ))),
                PoolError::PoolSetupError(msg) => {
                    Err(Error::Internal(format!("Pool setup error: {}", msg)))
                }
                PoolError::WorkerError(msg) => {
                    Err(Error::Internal(format!("Worker error: {}", msg)))
                }
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
            }
        }
    }
}
