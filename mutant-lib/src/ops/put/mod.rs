use crate::error::Error;
use crate::index::{PadInfo, PadStatus};
use crate::internal_events::invoke_put_callback;
use crate::network::{Network, NetworkError};
use crate::ops::worker::{self, AsyncTask, PoolError, WorkerPoolConfig};
use crate::ops::MAX_CONFIRMATION_DURATION;
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error, info, warn};
use mutant_protocol::{PutCallback, PutEvent, StorageMode};
use std::{ops::Range, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio::time::Instant;

use super::{
    DATA_ENCODING_PRIVATE_DATA, DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX,
    PAD_RECYCLING_RETRIES,
};

#[derive(Clone)]
struct Context {
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: Arc<String>,
    data: Arc<Vec<u8>>,
    chunk_ranges: Arc<Vec<Range<usize>>>,
    public: bool,
}

/// Update a key with new content, preserving the public index pad if applicable.
///
/// For public keys, this function ensures that the public index pad is preserved during updates.
/// This is critical because the public index pad must remain the same to maintain accessibility
/// of the key through its public address. The function:
/// 1. Extracts and preserves the index pad from the existing public key
/// 2. Removes the key (which moves all pads to free_pads or pending_verification_pads)
/// 3. Creates a new key with the updated content
/// 4. Replaces the newly created index pad with the preserved one
/// 5. Updates the index pad data to reflect the new content
async fn update(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    content: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    info!("Update for {}", key_name);

    // Special handling for public keys to preserve the index pad
    let mut preserved_index_pad = None;

    // Check if this is a public key and extract the index pad before removing the key
    if public && index.read().await.is_public(key_name) {
        info!("Preserving public index pad for key {}", key_name);
        preserved_index_pad = index.read().await.extract_public_index_pad(key_name);
    }

    // Remove the key (this will move all pads to free_pads or pending_verification_pads)
    index.write().await.remove_key(key_name).unwrap();

    // Create a new key with the updated content
    let result = first_store(
        index.clone(),
        network.clone(),
        key_name,
        content.clone(),
        mode,
        public,
        no_verify,
        put_callback.clone(),
    )
    .await?;

    // If we preserved an index pad, replace the newly created one with it
    if let Some(old_index_pad) = preserved_index_pad {
        info!("Replacing new index pad with preserved one for key {}", key_name);

        // Get the data pads from the newly created key
        let data_pads = if let Some(entry) = index.read().await.get_entry(key_name) {
            if let crate::index::master_index::IndexEntry::PublicUpload(_, data_pads) = entry {
                data_pads.clone()
            } else {
                return Err(Error::Internal(format!(
                    "Expected PublicUpload entry for key {}, but found PrivateKey",
                    key_name
                )));
            }
        } else {
            return Err(Error::Internal(format!(
                "Key {} not found after first_store",
                key_name
            )));
        };

        // Update the key with the preserved index pad
        index.write().await.update_public_key_with_preserved_index_pad(
            key_name,
            old_index_pad,
            data_pads,
        )?;

        // Regenerate the index pad data
        let (index_pad, index_data) = index.write().await.populate_index_pad(key_name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        // Create a context for writing the index pad
        let index_pad_context = Context {
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(key_name.to_string()),
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public,
        };

        // Write the index pad
        write_pipeline(
            index_pad_context,
            vec![index_pad],
            no_verify,
            put_callback.clone(),
        )
        .await?;
    }

    Ok(result)
}

pub(super) async fn put(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    content: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    if index.read().await.contains_key(key_name) {
        if index
            .read()
            .await
            .verify_checksum(key_name, &content, mode.clone())
        {
            info!("Resume for {}", key_name);
            resume(
                index,
                network,
                key_name,
                content,
                mode,
                public,
                no_verify,
                put_callback,
            )
            .await
        } else {
            // Call the dedicated update function
            update(
                index,
                network,
                key_name,
                content,
                mode,
                public,
                no_verify,
                put_callback,
            )
            .await
        }
    } else {
        info!("First store for {}", key_name);
        first_store(
            index,
            network,
            key_name,
            content,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await
    }
}

async fn resume(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let pads = index.read().await.get_pads(name);

    if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
        index.write().await.remove_key(name).unwrap();
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await;
    }

    let chunk_ranges = index.read().await.chunk_data(&data_bytes, mode.clone());

    if pads.len() != chunk_ranges.len() {
        warn!(
            "Resuming key '{}' with data size mismatch. Index has {} pads, current data requires {}. Forcing rewrite.",
            name,
            pads.len(),
            chunk_ranges.len()
        );
        index.write().await.remove_key(name)?;
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback.clone(),
        )
        .await;
    }

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        data: data_bytes.clone(),
        chunk_ranges: Arc::new(chunk_ranges),
        public,
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback.clone()).await?;

    if public {
        let (index_pad, index_data) = index.write().await.populate_index_pad(name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        let index_pad_context = Context {
            // Reuse index and network Arcs
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(name.to_string()), // Reuse name Arc
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public, // Keep public flag
                    // index_pad_data is None for the index pad itself
        };

        // Call write_pipeline again for the single index pad
        write_pipeline(
            index_pad_context,
            vec![index_pad],
            no_verify,
            put_callback.clone(), // Clone the callback Arc again
        )
        .await?;
    }

    Ok(pads[0].address)
}

async fn first_store(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let (pads, chunk_ranges) = index
        .write()
        .await
        .create_key(name, &data_bytes, mode, public)?;

    info!("Created key {} with {} pads", name, pads.len());

    let address = pads[0].address;

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        chunk_ranges: Arc::new(chunk_ranges),
        data: data_bytes.clone(),
        public,
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback.clone()).await?;

    if public {
        let (index_pad, index_data) = index.write().await.populate_index_pad(name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        let index_pad_context = Context {
            // Reuse index and network Arcs
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(name.to_string()), // Reuse name Arc
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public, // Keep public flag
                    // index_pad_data is None for the index pad itself
        };

        // Call write_pipeline again for the single index pad
        write_pipeline(
            index_pad_context,
            vec![index_pad],
            no_verify,
            put_callback.clone(), // Clone the callback Arc again
        )
        .await?;
    }

    // Final completion callback after all pipelines are done
    invoke_put_callback(&put_callback, PutEvent::Complete)
        .await
        .unwrap();

    Ok(address)
}

struct PutTaskContext {
    base_context: Context,
    no_verify: Arc<bool>,
    put_callback: Option<PutCallback>,
}

#[derive(Clone)]
struct PutTaskProcessor {
    context: Arc<PutTaskContext>,
}

impl PutTaskProcessor {
    fn new(context: Arc<PutTaskContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl AsyncTask<PadInfo, PutTaskContext, Object<crate::network::client::ClientManager>, (), Error>
    for PutTaskProcessor
{
    type ItemId = usize;

    async fn process(
        &self,
        worker_id: usize,
        client: &Object<crate::network::client::ClientManager>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, ()), (Error, PadInfo)> {
        let mut pad_state = pad.clone();
        let current_pad_address = pad_state.address;
        let initial_status = pad_state.status;
        let mut put_succeeded = false;
        let is_public = self.context.base_context.public;

        let should_put = match initial_status {
            PadStatus::Generated | PadStatus::Free => true,
            PadStatus::Written => false,
            PadStatus::Confirmed => {
                return Ok((pad_state.chunk_index, ()));
            }
        };

        if should_put {
            // Determine if this is an index pad for a public upload
            // For public uploads, the index pad is the one with chunk_index 0 and only one chunk range
            let is_index_pad = is_public
                && pad_state.chunk_index == 0
                && self.context.base_context.chunk_ranges.len() == 1;

            let data_encoding = if is_public {
                if is_index_pad {
                    debug!(
                        "Using PUBLIC_INDEX encoding for index pad {} (chunk_index={})",
                        pad_state.address, pad_state.chunk_index
                    );
                    DATA_ENCODING_PUBLIC_INDEX
                } else {
                    debug!(
                        "Using PUBLIC_DATA encoding for data pad {} (chunk_index={})",
                        pad_state.address, pad_state.chunk_index
                    );
                    DATA_ENCODING_PUBLIC_DATA
                }
            } else {
                debug!(
                    "Using PRIVATE_DATA encoding for pad {} (chunk_index={})",
                    pad_state.address, pad_state.chunk_index
                );
                DATA_ENCODING_PRIVATE_DATA
            };

            let chunk_index = pad_state.chunk_index;
            let range = self
                .context
                .base_context
                .chunk_ranges
                .get(chunk_index)
                .ok_or_else(|| {
                    (
                        Error::Internal(format!(
                            "Invalid chunk index {} for key {}",
                            chunk_index, self.context.base_context.name
                        )),
                        pad_state.clone(),
                    )
                })?;
            let chunk_data = self
                .context
                .base_context
                .data
                .get(range.clone())
                .ok_or_else(|| {
                    (
                        Error::Internal(format!(
                            "Data range {:?} out of bounds for key {}",
                            range, self.context.base_context.name
                        )),
                        pad_state.clone(),
                    )
                })?;

            let max_put_retries = PAD_RECYCLING_RETRIES;
            let mut last_put_error: Option<Error> = None;
            for attempt in 1..=max_put_retries {
                let put_result = self
                    .context
                    .base_context
                    .network
                    .put(client, &pad_state, chunk_data, data_encoding, is_public)
                    .await;

                match put_result {
                    Ok(_) => {
                        // Check if this was a Generated pad that needs a PadReserved event
                        let was_generated = initial_status == PadStatus::Generated;

                        pad_state.status = PadStatus::Written;
                        match self
                            .context
                            .base_context
                            .index
                            .write()
                            .await
                            .update_pad_status(
                                &self.context.base_context.name,
                                &current_pad_address,
                                PadStatus::Written,
                                None,
                            ) {
                            Ok(updated_pad) => pad_state = updated_pad,
                            Err(e) => return Err((e, pad_state.clone())),
                        }

                        // If the pad was in Generated status, send PadReserved event
                        if was_generated {
                            info!(
                                "Worker {} sending PadReserved event for pad {} (chunk {})",
                                worker_id, current_pad_address, pad_state.chunk_index
                            );
                            invoke_put_callback(&self.context.put_callback, PutEvent::PadReserved)
                                .await
                                .map_err(|e| (e, pad_state.clone()))?;
                        }

                        put_succeeded = true;
                        last_put_error = None;
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Worker {} failed put attempt {}/{} for pad {} (chunk {}): {}. Retrying...",
                            worker_id, attempt, max_put_retries, current_pad_address, pad_state.chunk_index, e
                        );
                        last_put_error = Some(Error::Network(e));
                        if attempt < max_put_retries {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }

            if !put_succeeded {
                return Err((
                    last_put_error.unwrap_or_else(|| {
                        Error::Internal(format!(
                            "Put failed for pad {} after {} retries with unknown error",
                            current_pad_address, max_put_retries
                        ))
                    }),
                    pad_state,
                ));
            }

            invoke_put_callback(&self.context.put_callback, PutEvent::PadsWritten)
                .await
                .map_err(|e| (e, pad_state.clone()))?;
        } else {
            put_succeeded = true;
            pad_state = pad.clone();
        }

        if put_succeeded && !*self.context.no_verify {
            let confirmation_start = Instant::now();
            let max_duration = MAX_CONFIRMATION_DURATION;
            let mut confirmation_succeeded = false;

            while confirmation_start.elapsed() < max_duration {
                let owned_key;
                let secret_key_ref = if is_public {
                    None
                } else {
                    owned_key = pad_state.secret_key();
                    Some(&owned_key)
                };
                match self
                    .context
                    .base_context
                    .network
                    .get(client, &current_pad_address, secret_key_ref)
                    .await
                {
                    Ok(get_result) => {
                        let checksum_match = pad.checksum == PadInfo::checksum(&get_result.data);
                        let counter_match = pad.last_known_counter == get_result.counter;
                        let size_match = pad.size == get_result.data.len();
                        if checksum_match && counter_match && size_match {
                            pad_state.status = PadStatus::Confirmed;
                            match self
                                .context
                                .base_context
                                .index
                                .write()
                                .await
                                .update_pad_status(
                                    &self.context.base_context.name,
                                    &current_pad_address,
                                    PadStatus::Confirmed,
                                    None,
                                ) {
                                Ok(final_pad) => {
                                    confirmation_succeeded = true;
                                    pad_state = final_pad;
                                    break;
                                }
                                Err(e) => {
                                    warn!("Worker {} failed to update index status to Confirmed for pad {}: {}. Retrying confirmation...", worker_id, current_pad_address, e);
                                }
                            }
                        }
                    }
                    Err(NetworkError::GetError(ant_networking::GetRecordError::RecordNotFound)) => {
                        debug!(
                            "Worker {} confirming pad {}, not found yet. Retrying...",
                            worker_id, current_pad_address
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Worker {} encountered network error while confirming pad {}: {}. Retrying confirmation...",
                            worker_id, current_pad_address, e
                        );
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            if !confirmation_succeeded {
                error!(
                    "Worker {} failed to confirm pad {} within {:?}. Returning error.",
                    worker_id, current_pad_address, max_duration
                );
                return Err((
                    Error::Internal(format!("Confirmation timeout: {}", current_pad_address)),
                    pad_state,
                ));
            }

            invoke_put_callback(&self.context.put_callback, PutEvent::PadsConfirmed)
                .await
                .map_err(|e| (e, pad_state.clone()))?;
        }

        Ok((pad_state.chunk_index, ()))
    }
}

// Define the recycling logic function
async fn recycle_put_pad(
    context: Context,
    error_cause: Error,
    pad_to_recycle: PadInfo,
) -> Result<Option<PadInfo>, Error> {
    warn!(
        "Recycling pad {} for key '{}' due to error: {:?}",
        pad_to_recycle.address, context.name, error_cause
    );

    // Log the pad status before recycling
    debug!(
        "Pad to recycle: address={}, status={:?}, chunk_index={}, size={}",
        pad_to_recycle.address, pad_to_recycle.status, pad_to_recycle.chunk_index, pad_to_recycle.size
    );

    match context
        .index
        .write()
        .await
        .recycle_errored_pad(&context.name, &pad_to_recycle.address)
        .await
    {
        Ok(new_pad) => {
            debug!(
                "Successfully recycled pad {} -> {} for key '{}', returning to pool. New pad status: {:?}",
                pad_to_recycle.address, new_pad.address, context.name, new_pad.status
            );

            // Return the new pad to be processed by the worker pool
            Ok(Some(new_pad))
        }
        Err(recycle_err) => {
            error!(
                "Failed to recycle pad {} for key '{}': {}. Skipping this pad.",
                pad_to_recycle.address, context.name, recycle_err
            );
            // We'll just skip this pad and log the error rather than halting the entire process
            Ok(None)
        }
    }
}

async fn write_pipeline(
    context: Context,
    pads: Vec<PadInfo>,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<(), Error> {
    let key_name = context.name.clone();

    // Count pads by status before filtering
    let total_chunks = pads.len();
    let initial_written_count = pads
        .iter()
        .filter(|p| p.status == PadStatus::Written)
        .count();
    let initial_confirmed_count = pads
        .iter()
        .filter(|p| p.status == PadStatus::Confirmed)
        .count();
    let chunks_to_reserve = pads
        .iter()
        .filter(|p| p.status == PadStatus::Generated)
        .count();

    // Send Starting event with pad counts
    info!(
        "Sending Starting event for key '{}': total={}, written={}, confirmed={}, to_reserve={}",
        key_name,
        total_chunks,
        initial_written_count,
        initial_confirmed_count,
        chunks_to_reserve
    );

    invoke_put_callback(
        &put_callback,
        PutEvent::Starting {
            total_chunks,
            initial_written_count,
            initial_confirmed_count,
            chunks_to_reserve,
        },
    )
    .await
    .map_err(|e| Error::Internal(format!("Callback error on Starting event: {:?}", e)))?;

    // Filter out already confirmed pads - these don't need processing
    let pads_to_process: Vec<PadInfo> = pads
        .into_iter()
        .filter(|p| p.status != PadStatus::Confirmed)
        .collect();
    let initial_process_count = pads_to_process.len();

    if initial_process_count == 0 {
        info!("All pads for key '{}' already confirmed.", key_name);
        // Invoke Complete callback immediately if nothing to do?
        invoke_put_callback(&put_callback, PutEvent::Complete)
            .await
            .map_err(|e| Error::Internal(format!("Callback error: {:?}", e)))?;
        return Ok(());
    }

    // 1. Create Context for Task Processor
    let put_task_context = Arc::new(PutTaskContext {
        base_context: context.clone(), // Clone base context Arc
        no_verify: Arc::new(no_verify),
        put_callback: put_callback.clone(),
    });

    // 2. Create Task Processor
    let task_processor = PutTaskProcessor::new(put_task_context.clone());

    // 3. Create WorkerPoolConfig
    let config = WorkerPoolConfig {
        network: context.network.clone(), // Clone network Arc
        client_config: crate::network::client::Config::Put, // Use crate path
        task_processor,
        enable_recycling: true, // Ensure recycling is enabled for PUT
        total_items_hint: initial_process_count,
    };

    debug!(
        "Created WorkerPoolConfig for PUT with recycling enabled, total_items_hint={}",
        initial_process_count
    );

    // Define the recycling closure
    let recycle_fn = {
        let context_clone = context.clone(); // Clone context for the closure
        let key_name_for_log = context.name.to_string(); // Clone the key name for logging

        Arc::new(move |error: Error, pad: PadInfo| {
            let context_inner = context_clone.clone(); // Clone again for the async block
            let key_name_inner = key_name_for_log.clone(); // Clone for the async block

            info!(
                "Creating recycling function for key '{}', pad {}",
                key_name_inner, pad.address
            );

            Box::pin(async move {
                info!(
                    "Executing recycling function for key '{}', pad {}",
                    key_name_inner, pad.address
                );
                recycle_put_pad(context_inner, error, pad).await
            }) as futures::future::BoxFuture<'static, Result<Option<PadInfo>, Error>>
        })
    };

    // 4. Build WorkerPool
    let pool = match worker::build(config, Some(recycle_fn.clone())).await {
        Ok(pool) => pool,
        Err(e) => {
            error!(
                "Failed to build worker pool for PUT '{}': {:?}",
                key_name, e
            );
            return match e {
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
                _ => Err(Error::Internal(format!("Pool build failed: {:?}", e))),
            };
        }
    };

    // 5. Send pads to the pool
    if let Err(e) = pool.send_items(pads_to_process).await {
        error!(
            "Failed to send initial pads to worker pool for PUT '{}': {:?}",
            key_name, e
        );
        // If sending fails, the pool might be in a bad state.
        return match e {
            PoolError::PoolSetupError(msg) => Err(Error::Internal(msg)),
            // Other PoolErrors might be relevant here depending on send_items implementation
            _ => Err(Error::Internal(format!("Pool send_items failed: {:?}", e))),
        };
    }

    // 6. Run the Worker Pool (recycling is now internal, driven by passed fn)
    // Make sure to pass the recycle_fn to ensure the recycling mechanism is active
    let pool_result = pool.run(Some(recycle_fn)).await;

    // 7. Process Pool Results
    match pool_result {
        Ok(_results) => {
            // For PUT, successful completion of the pool.run() without error is the main success signal,
            // assuming the recycler handled intermediate task errors.
            // We could potentially verify the final state in the index here if needed.
            info!("PUT operation seemingly successful for key '{}'. Final state verification might be needed.", key_name);
            // Invoke final completion callback
            invoke_put_callback(&put_callback, PutEvent::Complete)
                .await
                .map_err(|e| Error::Internal(format!("Callback error: {:?}", e)))?;
            Ok(())
        }
        Err(pool_error) => {
            error!(
                "PUT worker pool failed for key '{}': {:?}",
                key_name, pool_error
            );
            // Map PoolError to crate::Error
            match pool_error {
                PoolError::TaskError(task_err) => Err(task_err), // Task error that couldn't be recycled
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
