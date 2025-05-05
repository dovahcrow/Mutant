use crate::error::Error;
use crate::events::{GetCallback, GetEvent};
use crate::index::{master_index::MasterIndex, PadInfo};
use crate::internal_events::invoke_get_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError};
use crate::ops::worker::{self, AsyncTask, PoolError, WorkerPoolConfig};
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error, warn};
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

use super::{DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX};

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

    debug!(
        "get_public: Processing pad {} with data_encoding={} (PUBLIC_INDEX={}, PUBLIC_DATA={})",
        address,
        index_pad_data.data_encoding,
        DATA_ENCODING_PUBLIC_INDEX,
        DATA_ENCODING_PUBLIC_DATA
    );

    match index_pad_data.data_encoding {
        DATA_ENCODING_PUBLIC_INDEX => {
            debug!("get_public: Found PUBLIC_INDEX pad, deserializing index");
            let index: Vec<PadInfo> = serde_cbor::from_slice(&index_pad_data.data)
                .map_err(|e| Error::Internal(format!("Failed to decode public index: {}", e)))?;

            debug!("get_public: Index contains {} data pads", index.len());

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

            debug!("get_public: Fetching data pads");
            fetch_pads_data(network, index, true, callback).await
        }
        DATA_ENCODING_PUBLIC_DATA => {
            debug!("get_public: Found PUBLIC_DATA pad, returning data directly");
            invoke_get_callback(&callback, GetEvent::Starting { total_chunks: 1000 })
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

// Context for the GET AsyncTask - REMOVED (or simplified)
// #[derive(Clone)]
// struct GetContext { ... }

// Task processor for GET operations.
#[derive(Clone)] // Required by WorkerPool
struct GetTaskProcessor {
    // Keep fields needed by process method
    network: Arc<Network>,
    public: bool,
    get_callback: Option<GetCallback>,
    // Remove fields related to old distribution logic
    // completion_notifier: Arc<Notify>,
    // total_items: Arc<std::sync::atomic::AtomicUsize>,
    // fetched_items_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl GetTaskProcessor {
    // Update constructor
    fn new(network: Arc<Network>, public: bool, get_callback: Option<GetCallback>) -> Self {
        Self {
            network,
            public,
            get_callback,
        }
    }
}

// Use () for Context generic as it's no longer stored in the pool
#[async_trait]
impl AsyncTask<PadInfo, (), Object<crate::network::client::ClientManager>, Vec<u8>, Error>
    for GetTaskProcessor
{
    type ItemId = usize; // Use chunk index for ordering

    async fn process(
        &self,
        _worker_id: usize, // worker_id not used
        client: &Object<crate::network::client::ClientManager>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, Vec<u8>), (Error, PadInfo)> {
        let mut retries_left = 5;
        let owned_key;
        let secret_key_ref = if self.public {
            None
        } else {
            owned_key = pad.secret_key();
            Some(&owned_key)
        };

        loop {
            match self
                .network // Access directly from self
                .get(client, &pad.address, secret_key_ref)
                .await
            {
                Ok(get_result) => {
                    let checksum_match = pad.checksum == PadInfo::checksum(&get_result.data);
                    let counter_match = pad.last_known_counter == get_result.counter;
                    let size_match = pad.size == get_result.data.len();
                    if checksum_match && counter_match && size_match {
                        // Invoke callback directly
                        invoke_get_callback(&self.get_callback, GetEvent::PadFetched)
                            .await
                            .map_err(|e| (e, pad.clone()))?;

                        return Ok((pad.chunk_index, get_result.data));
                    }
                }
                Err(e) => match e {
                    _ => {}
                },
            }

            retries_left -= 1;

            warn!(
                "GET failed for pad {} (chunk {}). Retries left: {}",
                pad.address, pad.chunk_index, retries_left
            );

            if retries_left <= 0 {
                return Err((
                    Error::Internal(format!(
                        "GET failed for pad {} (chunk {}) after {} retries",
                        pad.address, pad.chunk_index, 5
                    )),
                    pad,
                ));
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
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
    debug!(
        "fetch_pads_data: Starting to fetch {} pads, public={}",
        total_pads_to_fetch, public
    );

    if total_pads_to_fetch == 0 {
        debug!("fetch_pads_data: No pads to fetch, returning empty data");
        invoke_get_callback(&get_callback, GetEvent::Complete)
            .await
            .unwrap();
        return Ok(Vec::new());
    }

    // Log the first few pads for debugging
    for (i, pad) in pads.iter().take(3).enumerate() {
        debug!(
            "fetch_pads_data: Pad[{}]: address={}, chunk_index={}, size={}",
            i, pad.address, pad.chunk_index, pad.size
        );
    }
    if pads.len() > 3 {
        debug!("fetch_pads_data: ... and {} more pads", pads.len() - 3);
    }

    // 1. Create Task Processor (directly)
    let task_processor = GetTaskProcessor::new(network.clone(), public, get_callback.clone());

    // 2. Create WorkerPoolConfig (no Context)
    let config = WorkerPoolConfig {
        network,
        client_config: crate::network::client::Config::Get,
        task_processor,
        enable_recycling: false, // No recycling for GET
        total_items_hint: total_pads_to_fetch,
    };

    // 3. Build WorkerPool (no recycle_fn)
    let pool = match worker::build(config, None).await {
        // Pass None for recycle_fn
        Ok(pool) => pool,
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

    // 4. Send pads to the pool
    if let Err(e) = pool.send_items(pads).await {
        // Send pads directly
        error!("Failed to send pads to worker pool for GET: {:?}", e);
        return match e {
            PoolError::PoolSetupError(msg) => Err(Error::Internal(msg)),
            _ => Err(Error::Internal(format!("Pool send_items failed: {:?}", e))),
        };
    }

    // REMOVED Distribution Task
    // let send_pads_task = { ... };

    // 5. Run the Worker Pool (no recycle_fn)
    debug!(
        "fetch_pads_data: Running worker pool to fetch {} pads",
        total_pads_to_fetch
    );
    let pool_run_result = pool.run(None).await; // Pass None for recycle_fn
    debug!("fetch_pads_data: Worker pool run completed");

    // REMOVED Awaiting Distribution Task
    // if let Err(e) = send_pads_task.await { ... };

    // 6. Process Results
    match pool_run_result {
        Ok(mut fetched_results) => {
            debug!(
                "fetch_pads_data: Got {} results from worker pool",
                fetched_results.len()
            );
            if fetched_results.len() != total_pads_to_fetch {
                warn!(
                    "GET result count mismatch: expected {}, got {}. Some pads might have failed.",
                    total_pads_to_fetch,
                    fetched_results.len()
                );
                return Err(Error::Internal(format!(
                    "GET failed: Fetched {} pads, expected {}",
                    fetched_results.len(),
                    total_pads_to_fetch
                )));
            }

            debug!("fetch_pads_data: Sorting results by chunk_index");
            fetched_results.sort_by_key(|(chunk_index, _)| *chunk_index);

            debug!("fetch_pads_data: Collecting data from all chunks");
            let collected_data: Vec<Vec<u8>> =
                fetched_results.into_iter().map(|(_, data)| data).collect();

            let final_capacity: usize = collected_data.iter().map(|data| data.len()).sum();
            debug!(
                "fetch_pads_data: Assembling final data with capacity {}",
                final_capacity
            );

            let mut final_data: Vec<u8> = Vec::with_capacity(final_capacity);
            for (i, pad_data) in collected_data.iter().enumerate() {
                debug!(
                    "fetch_pads_data: Adding chunk {} with size {}",
                    i,
                    pad_data.len()
                );
                final_data.extend(pad_data);
            }

            debug!("fetch_pads_data: Final data size: {}", final_data.len());
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
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
            }
        }
    }
}
