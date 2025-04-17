use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::events::{invoke_put_callback, PutCallback, PutEvent};
use crate::index::structure::PadStatus;
use crate::index::{IndexManager, PadInfo};
use crate::pad_lifecycle::PadLifecycleManager;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::{StorageError, StorageManager};
use autonomi::SecretKey;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio;
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::common::{
    DataManagerDependencies, WriteTaskInput, CONFIRMATION_RETRY_DELAY, CONFIRMATION_RETRY_LIMIT,
};

pub(crate) async fn store_op(
    deps: &DataManagerDependencies,
    user_key: String,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!("DataOps: Starting store operation for key '{}'", user_key);
    let callback_arc = Arc::new(Mutex::new(callback));
    let data_size = data_bytes.len();

    let chunk_size = deps.index_manager.get_scratchpad_size().await?;
    if chunk_size == 0 {
        error!("Configured scratchpad size is 0. Cannot proceed with chunking.");
        return Err(DataError::ChunkingError(
            "Invalid scratchpad size (0) configured".to_string(),
        ));
    }
    let chunks = chunk_data(data_bytes, chunk_size)?;
    debug!(
        "Data chunked into {} pieces using size {}.",
        chunks.len(),
        chunk_size
    );

    let (_prepared_key_info, write_tasks_input) = crate::pad_lifecycle::prepare_pads_for_store(
        deps,
        &user_key,
        data_size,
        &chunks,
        callback_arc.clone(),
    )
    .await?;

    if write_tasks_input.is_empty() {
        info!("Store operation for key '{}' requires no write tasks (already complete or empty data).", user_key);

        return Ok(());
    }

    let execution_result = execute_write_confirm_tasks(
        deps.clone(),
        user_key.clone(),
        write_tasks_input,
        callback_arc.clone(),
    )
    .await;

    if let Err(err) = execution_result {
        warn!(
            "Store operation execution failed for key '{}': {}",
            user_key, err
        );

        return Err(err);
    }

    let final_key_info = deps
        .index_manager
        .get_key_info(&user_key)
        .await?
        .ok_or_else(|| {
            DataError::InternalError(format!(
                "KeyInfo for '{}' disappeared unexpectedly after processing",
                user_key
            ))
        })?;

    let all_pads_confirmed = final_key_info
        .pads
        .iter()
        .all(|p| p.status == PadStatus::Confirmed);

    if all_pads_confirmed && !final_key_info.is_complete {
        debug!(
            "All pads for key '{}' are now Confirmed. Marking key as complete.",
            user_key
        );

        deps.index_manager.mark_key_complete(&user_key).await?;
    } else if !all_pads_confirmed {
        warn!("Store operation for key '{}' finished processing tasks, but not all pads reached Confirmed state. Key remains incomplete.", user_key);
    }

    info!(
        "Store operation processing finished for key '{}'. Final state: {}",
        user_key,
        if all_pads_confirmed {
            "Complete"
        } else {
            "Incomplete"
        }
    );

    if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
        .await
        .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        warn!("Operation cancelled by callback after completion.");
    }

    Ok(())
}

struct ConfirmContext {
    index_manager: Arc<dyn IndexManager>,
    pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn crate::network::NetworkAdapter>,
}

async fn confirm_pad_write(
    ctx: ConfirmContext,
    user_key: String,
    pad_info: PadInfo,
    pad_secret_key: SecretKey,
    expected_chunk_size: usize,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<usize, DataError> {
    let mut last_error: Option<DataError> = None;
    for attempt in 0..CONFIRMATION_RETRY_LIMIT {
        trace!(
            "Confirmation attempt {} for pad {}",
            attempt + 1,
            pad_info.address
        );

        let fetch_result = ctx
            .storage_manager
            .read_pad_scratchpad(&pad_info.address)
            .await;

        match fetch_result {
            Ok(scratchpad) => {
                let final_counter = scratchpad.counter();
                trace!(
                    "Attempt {}: Fetched scratchpad for pad {}. Final counter: {}",
                    attempt + 1,
                    pad_info.address,
                    final_counter
                );

                let counter_check_passed = match pad_info.origin {
                    PadOrigin::FreePool { initial_counter } => {
                        if final_counter > initial_counter {
                            trace!(
                                "Attempt {}: Counter check passed for FreePool pad {}.",
                                attempt + 1,
                                pad_info.address
                            );
                            true
                        } else {
                            trace!(
                                "Attempt {}: Counter check failed for pad {}. Initial: {}, Final: {}. Retrying...",
                                attempt + 1,
                                pad_info.address,
                                initial_counter,
                                final_counter
                            );

                            last_error = Some(DataError::InconsistentState(format!(
                                "Counter check failed after retries for pad {} ({} -> {})",
                                pad_info.address, initial_counter, final_counter
                            )));
                            false
                        }
                    }
                    PadOrigin::Generated => {
                        trace!(
                            "Attempt {}: Skipping counter check for Generated pad {}.",
                            attempt + 1,
                            pad_info.address
                        );
                        true
                    }
                };

                let mut data_check_passed = false;
                if counter_check_passed {
                    match scratchpad.decrypt_data(&pad_secret_key) {
                        Ok(decrypted_data) => {
                            if decrypted_data.len() == expected_chunk_size {
                                trace!(
                                    "Attempt {}: Data size check passed for pad {}. (Expected {}, Got {})",
                                    attempt + 1,
                                    pad_info.address,
                                    expected_chunk_size,
                                    decrypted_data.len()
                                );
                                data_check_passed = true;
                            } else {
                                warn!(
                                    "Attempt {}: Data size check failed for pad {}. Expected {}, Got {}. Retrying...",
                                    attempt + 1,
                                    pad_info.address,
                                    expected_chunk_size,
                                    decrypted_data.len()
                                );

                                last_error =
                                    Some(DataError::InconsistentState(format!(
                                    "Confirmed data size mismatch for pad {} (Expected {}, Got {})",
                                    pad_info.address, expected_chunk_size, decrypted_data.len()
                                )));
                            }
                        }
                        Err(e) => {
                            error!(
                                "Attempt {}: Failed to decrypt fetched data for pad {} during confirmation: {}. Retrying...",
                                attempt + 1,
                                pad_info.address,
                                e
                            );

                            last_error = Some(DataError::InternalError(format!(
                                "Confirmation decryption failed for {}: {}",
                                pad_info.address, e
                            )));
                        }
                    }
                }

                if counter_check_passed && data_check_passed {
                    match ctx
                        .index_manager
                        .update_pad_status(&user_key, &pad_info.address, PadStatus::Confirmed)
                        .await
                    {
                        Ok(_) => {
                            trace!("Updated status to Confirmed for pad {}", pad_info.address);

                            let network_choice = ctx.network_adapter.get_network_choice();
                            if let Err(e) = ctx
                                .pad_lifecycle_manager
                                .save_index_cache(network_choice)
                                .await
                            {
                                warn!(
                                    "Failed to save index cache after setting pad {} to Confirmed: {}. Proceeding anyway.",
                                    pad_info.address, e
                                );
                            }

                            if !invoke_put_callback(
                                &mut *callback_arc.lock().await,
                                PutEvent::ChunkConfirmed {
                                    chunk_index: pad_info.chunk_index,
                                },
                            )
                            .await
                            .map_err(|e| {
                                DataError::InternalError(format!(
                                    "Callback invocation failed: {}",
                                    e
                                ))
                            })? {
                                error!("Store operation cancelled by callback after chunk confirmation.");
                                return Err(DataError::OperationCancelled);
                            }

                            return Ok(pad_info.chunk_index);
                        }
                        Err(e) => {
                            error!(
                                "Failed to update pad status to Confirmed for {}: {}",
                                pad_info.address, e
                            );

                            return Err(DataError::Index(e));
                        }
                    }
                }
            }
            Err(fetch_err) => {
                let is_not_enough_copies = if let StorageError::Network(
                    crate::network::error::NetworkError::InternalError(msg),
                ) = &fetch_err
                {
                    msg.contains("NotEnoughCopies")
                } else {
                    false
                };

                if is_not_enough_copies {
                    trace!(
                        "Attempt {}: Transient fetch error for pad {}: {} Retrying...",
                        attempt + 1,
                        pad_info.address,
                        fetch_err
                    );

                    last_error = Some(DataError::Storage(fetch_err));
                } else {
                    error!(
                        "Confirmation failed for pad {}: Non-retriable storage error during fetch: {}",
                        pad_info.address, fetch_err
                    );

                    return Err(DataError::Storage(fetch_err));
                }
            }
        }

        if attempt < CONFIRMATION_RETRY_LIMIT - 1 {
            sleep(CONFIRMATION_RETRY_DELAY).await;
        }
    }

    error!(
        "Confirmation failed for pad {} after {} attempts.",
        pad_info.address, CONFIRMATION_RETRY_LIMIT
    );
    Err(last_error.unwrap_or_else(|| {
        DataError::InternalError(format!(
            "Confirmation timed out for pad {}",
            pad_info.address
        ))
    }))
}

async fn execute_write_confirm_tasks(
    deps: DataManagerDependencies,
    user_key: String,
    tasks_to_run_input: Vec<WriteTaskInput>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<(), DataError> {
    debug!(
        "Executing {} write/confirm tasks...",
        tasks_to_run_input.len()
    );
    let mut write_futures = FuturesUnordered::new();
    let mut confirm_futures = FuturesUnordered::new();

    let mut operation_error: Option<DataError> = None;
    let mut callback_cancelled = false;
    let mut pending_writes = 0;
    let mut pending_confirms = 0;

    let index_manager = Arc::clone(&deps.index_manager);
    let network_adapter = Arc::clone(&deps.network_adapter);
    let storage_manager = Arc::clone(&deps.storage_manager);
    let pad_lifecycle_manager = Arc::clone(&deps.pad_lifecycle_manager);

    for task_input in tasks_to_run_input {
        let storage_manager_clone = Arc::clone(&storage_manager);
        let secret_key_write = task_input.secret_key.clone();
        let pad_info_write = task_input.pad_info.clone();
        let current_status = pad_info_write.status.clone();
        let chunk_data = task_input.chunk_data;
        let expected_chunk_size = chunk_data.len();

        pending_writes += 1;
        write_futures.push(async move {
            let mut last_write_error: Option<DataError> = None;
            for attempt in 0..CONFIRMATION_RETRY_LIMIT {
                trace!(
                    "Write attempt {} for chunk {}, pad {}",
                    attempt + 1,
                    pad_info_write.chunk_index,
                    pad_info_write.address
                );
                let write_attempt_result = storage_manager_clone
                    .write_pad_data(&secret_key_write, &chunk_data, &current_status)
                    .await
                    .map(|_| ())
                    .map_err(DataError::Storage);

                match write_attempt_result {
                    Ok(()) => {
                        last_write_error = None;
                        break;
                    }
                    Err(e) => {
                        let is_retriable_network_error =
                            if let DataError::Storage(StorageError::Network(net_err)) = &e {
                                let msg = net_err.to_string();
                                msg.contains("NotEnoughCopies") || msg.contains("Timeout")
                            } else {
                                false
                            };

                        if is_retriable_network_error {
                            warn!(
                                "Write attempt {} failed for chunk {}, pad {}: {}. Retrying...",
                                attempt + 1,
                                pad_info_write.chunk_index,
                                pad_info_write.address,
                                e
                            );
                            last_write_error = Some(e);
                            if attempt < CONFIRMATION_RETRY_LIMIT - 1 {
                                sleep(CONFIRMATION_RETRY_DELAY).await;
                                continue;
                            } else {
                                break;
                            }
                        } else {
                            last_write_error = Some(e);
                            break;
                        }
                    }
                }
            }

            let final_write_result = match last_write_error {
                Some(err) => Err(err),
                None => Ok(()),
            };

            (
                pad_info_write,
                secret_key_write,
                expected_chunk_size,
                final_write_result,
            )
        });
    }

    debug!(
        "Processing {} pending writes and {} pending confirms.",
        pending_writes, pending_confirms
    );

    while (pending_writes > 0 || pending_confirms > 0) && operation_error.is_none() {
        tokio::select! {
            biased;


            Some((completed_pad_info, completed_secret_key, completed_chunk_size, write_result)) = write_futures.next(), if pending_writes > 0 => {
                pending_writes -= 1;
                match write_result {
                    Ok(()) => {
                        trace!(
                            "Write successful for chunk {}, pad {}",
                            completed_pad_info.chunk_index,
                            completed_pad_info.address
                        );


                        let callback_arc_clone = Arc::clone(&callback_arc);
                        if !invoke_put_callback(
                            &mut *callback_arc_clone.lock().await,
                            PutEvent::PadReserved { count: 1 },
                        )
                        .await
                        .map_err(|e| {
                            DataError::InternalError(format!(
                                "Callback invocation failed for PadReserved: {}",
                                e
                            ))
                        })? {
                            error!("Store operation cancelled by callback after pad reservation (write success).");
                            operation_error = Some(DataError::OperationCancelled);
                            callback_cancelled = true;
                            continue;
                        }


                        let index_manager_clone = Arc::clone(&index_manager);
                        let user_key_clone = user_key.clone();
                        let completed_pad_address = completed_pad_info.address;
                        match index_manager_clone
                            .update_pad_status(&user_key_clone, &completed_pad_address, PadStatus::Written)
                            .await
                        {
                            Ok(_) => {
                                trace!(
                                    "Updated status to Written for pad {}",
                                    completed_pad_address
                                );


                                let callback_arc_clone2 = Arc::clone(&callback_arc);
                                let chunk_index = completed_pad_info.chunk_index;
                                if !invoke_put_callback(
                                    &mut *callback_arc_clone2.lock().await,
                                    PutEvent::ChunkWritten { chunk_index },
                                )
                                .await
                                .map_err(|e| {
                                    DataError::InternalError(format!(
                                        "Callback invocation failed: {}",
                                        e
                                    ))
                                })? {
                                    error!("Store operation cancelled by callback after chunk write.");
                                    operation_error = Some(DataError::OperationCancelled);
                                    callback_cancelled = true;
                                    continue;
                                }


                                let index_manager_confirm = Arc::clone(&index_manager);
                                let pad_lifecycle_confirm = Arc::clone(&pad_lifecycle_manager);
                                let storage_manager_confirm = Arc::clone(&storage_manager);
                                let network_adapter_confirm = Arc::clone(&network_adapter);
                                let user_key_clone_confirm = user_key.clone();
                                let callback_arc_clone_confirm = callback_arc.clone();
                                let pad_info_confirm = completed_pad_info.clone();
                                let pad_key_confirm = completed_secret_key.clone();
                                let expected_chunk_size_confirm = completed_chunk_size;
                                let confirm_ctx = ConfirmContext {
                                    index_manager: Arc::clone(&index_manager_confirm),
                                    pad_lifecycle_manager: Arc::clone(&pad_lifecycle_confirm),
                                    storage_manager: Arc::clone(&storage_manager_confirm),
                                    network_adapter: Arc::clone(&network_adapter_confirm),
                                };

                                debug!(
                                    "Spawning confirmation task for pad {}",
                                    pad_info_confirm.address
                                );
                                pending_confirms += 1;
                                confirm_futures.push(tokio::spawn(async move {
                                    confirm_pad_write(
                                        confirm_ctx,
                                        user_key_clone_confirm,
                                        pad_info_confirm,
                                        pad_key_confirm,
                                        expected_chunk_size_confirm,
                                        callback_arc_clone_confirm,
                                    )
                                    .await
                                }));
                            }
                            Err(e) => {
                                error!(
                                    "Failed to update pad status to Written for {}: {}. Halting.",
                                    completed_pad_address,
                                    e
                                );
                                operation_error = Some(DataError::Index(e));
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to write chunk {} to pad {}: {}. Halting.",
                            completed_pad_info.chunk_index,
                            completed_pad_info.address,
                            e
                        );
                        if operation_error.is_none() {
                            operation_error = Some(e);
                        }
                        continue;
                    }
                }
            }


            Some(confirm_join_result) = confirm_futures.next(), if pending_confirms > 0 => {
                pending_confirms -= 1;

                match confirm_join_result {
                     Ok(Ok(confirmed_chunk_index)) => {
                        trace!("Confirmation task completed successfully for chunk index {}", confirmed_chunk_index);
                    }
                    Ok(Err(e)) => {
                         error!("Confirmation task failed: {}. Halting.", e);
                         if operation_error.is_none() {
                             operation_error = Some(e);
                         }
                         if matches!(operation_error, Some(DataError::OperationCancelled)) {
                            callback_cancelled = true;
                         }
                         continue;
                    }
                    Err(join_err) => {
                        error!("Confirmation task panicked or was cancelled: {}. Halting.", join_err);
                         if operation_error.is_none() {
                            operation_error = Some(DataError::InternalError(format!("Confirmation task failed unexpectedly: {}", join_err)));
                         }
                         continue;
                    }
                }
            }

            else => {

            }
        }
    }

    if callback_cancelled || operation_error.is_some() {
        debug!("Aborting remaining write/confirmation tasks due to error or cancellation.");
        while write_futures.next().await.is_some() {}
        while confirm_futures.next().await.is_some() {}
    }

    match operation_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
