// Store operation logic

use crate::data::chunking::chunk_data; // reassemble_data not used here, but keeping for potential future needs
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

// Define structure for tasks to be executed concurrently - REMOVED as unused
/*
enum PadTask {
    Write {
        pad_info: PadInfo,
        secret_key: SecretKey,
        chunk_data: Vec<u8>,
        is_new_hint: bool,
    },
}
*/

// --- Store Operation ---

pub(crate) async fn store_op(
    deps: &DataManagerDependencies,
    user_key: String, // Take ownership
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!("DataOps: Starting store operation for key '{}'", user_key);
    let callback_arc = Arc::new(Mutex::new(callback));
    let data_size = data_bytes.len();

    // Chunk data first (will be passed to preparation function)
    // Fetch the configured chunk size from the index manager
    let chunk_size = deps.index_manager.get_scratchpad_size().await?;
    if chunk_size == 0 {
        // Prevent division by zero or infinite looping if size is misconfigured
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

    // --- Preparation Phase ---
    // Call prepare_pads_for_store to handle KeyInfo fetching, resume/new logic,
    // pad acquisition/replacement, and persistence of the initial/updated KeyInfo.
    // It returns the KeyInfo state *before* writes begin and the list of tasks.
    let (_prepared_key_info, write_tasks_input) = crate::pad_lifecycle::prepare_pads_for_store(
        deps, // Pass reference
        &user_key,
        data_size,
        &chunks,
        callback_arc.clone(), // Clone Arc for the function
    )
    .await?;

    // Handle cases returned by prepare_pads_for_store:
    // 1. Operation already complete (resume path, key was already complete)
    // 2. Empty data upload (new path, num_chunks == 0)
    // In both cases, prepare_pads_for_store returns an empty task list.
    if write_tasks_input.is_empty() {
        info!("Store operation for key '{}' requires no write tasks (already complete or empty data).", user_key);
        // The Complete callback was already handled within prepare_pads_for_store for empty data.
        // For already complete resume, no callback needed here.
        return Ok(());
    }

    // --- Concurrent Processing (Combined Write & Confirm) ---
    // Delegate the execution loop to the helper function.
    let execution_result = execute_write_confirm_tasks(
        deps.clone(),         // Clone dependencies for the helper
        user_key.clone(),     // Clone user_key
        write_tasks_input,    // Pass the tasks
        callback_arc.clone(), // Clone callback Arc
    )
    .await;

    // Abort remaining futures logic MOVED inside execute_write_confirm_tasks

    // --- Final Check and Completion ---
    // If execution failed, return the error immediately.
    if let Err(err) = execution_result {
        warn!(
            "Store operation execution failed for key '{}': {}",
            user_key, err
        );
        // Don't send Complete callback if execution loop failed.
        return Err(err);
    }

    // Execution completed successfully, proceed with final checks.
    // Re-fetch the latest KeyInfo to check final status
    // Use the original user_key (String) here, or clone again if needed
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

    // Check if all pads are Confirmed
    // This logic uses the `prepared_key_info` which might be stale if errors occurred during execution.
    // Re-fetching `final_key_info` above provides the most up-to-date status.
    let all_pads_confirmed = final_key_info
        .pads
        .iter()
        .all(|p| p.status == PadStatus::Confirmed);

    if all_pads_confirmed && !final_key_info.is_complete {
        debug!(
            "All pads for key '{}' are now Confirmed. Marking key as complete.",
            user_key
        );
        // Use original deps here
        deps.index_manager.mark_key_complete(&user_key).await?;
    } else if !all_pads_confirmed {
        warn!("Store operation for key '{}' finished processing tasks, but not all pads reached Confirmed state. Key remains incomplete.", user_key);
    }

    // Send Complete callback regardless of whether marked complete, as processing finished.
    info!(
        "Store operation processing finished for key '{}'. Final state: {}",
        user_key,
        if all_pads_confirmed {
            "Complete"
        } else {
            "Incomplete"
        }
    );
    // Use original callback_arc
    if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
        .await
        .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        warn!("Operation cancelled by callback after completion.");
        // Don't return error here, the operation is done.
    }

    Ok(())
}

// --- Internal Helper Functions ---

/// Handles the confirmation logic for a single pad write.
/// Includes retry mechanism and counter checks for FreePool pads.
async fn confirm_pad_write(
    // Pass individual Arcs for clarity and potentially less overhead
    index_manager: Arc<dyn IndexManager>,
    pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn crate::network::NetworkAdapter>,
    user_key: String,
    pad_info: PadInfo,
    pad_secret_key: SecretKey,
    expected_chunk_size: usize,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<usize, DataError> {
    // --- Confirmation Retry Loop --- START
    let mut last_error: Option<DataError> = None;
    for attempt in 0..CONFIRMATION_RETRY_LIMIT {
        trace!(
            "Confirmation attempt {} for pad {}",
            attempt + 1,
            pad_info.address
        );
        // Fetch the scratchpad using the cloned storage manager
        let fetch_result = storage_manager.read_pad_scratchpad(&pad_info.address).await;

        match fetch_result {
            Ok(scratchpad) => {
                // Directly get the counter and perform the check
                let final_counter = scratchpad.counter();
                trace!(
                    "Attempt {}: Fetched scratchpad for pad {}. Final counter: {}",
                    attempt + 1,
                    pad_info.address,
                    final_counter
                );

                // Check counter increment for FreePool pads
                let counter_check_passed = match pad_info.origin {
                    PadOrigin::FreePool { initial_counter } => {
                        if final_counter > initial_counter {
                            trace!(
                                "Attempt {}: Counter check passed for FreePool pad {}.",
                                attempt + 1,
                                pad_info.address
                            );
                            true // Counter incremented
                        } else {
                            trace!(
                                "Attempt {}: Counter check failed for pad {}. Initial: {}, Final: {}. Retrying...",
                                attempt + 1,
                                pad_info.address,
                                initial_counter,
                                final_counter
                            );
                            // Store error in case loop finishes
                            last_error = Some(DataError::InconsistentState(format!(
                                "Counter check failed after retries for pad {} ({} -> {})",
                                pad_info.address, initial_counter, final_counter
                            )));
                            false // Counter did not increment yet
                        }
                    }
                    PadOrigin::Generated => {
                        trace!(
                            "Attempt {}: Skipping counter check for Generated pad {}.",
                            attempt + 1,
                            pad_info.address
                        );
                        true // No check needed for generated pads
                    }
                };

                let mut data_check_passed = false;
                if counter_check_passed {
                    // Counter check passed, now decrypt and check data size
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
                                // Store specific error for size mismatch
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
                            // Store decryption error
                            last_error = Some(DataError::InternalError(format!(
                                "Confirmation decryption failed for {}: {}",
                                pad_info.address, e
                            )));
                        }
                    }
                }

                // Proceed only if BOTH checks passed
                if counter_check_passed && data_check_passed {
                    // Counter check passed (or skipped), proceed to update status
                    match index_manager
                        .update_pad_status(&user_key, &pad_info.address, PadStatus::Confirmed)
                        .await
                    {
                        Ok(_) => {
                            trace!("Updated status to Confirmed for pad {}", pad_info.address);
                            // --- Persist Index Cache ---
                            let network_choice = network_adapter.get_network_choice();
                            if let Err(e) =
                                pad_lifecycle_manager.save_index_cache(network_choice).await
                            {
                                warn!(
                                    "Failed to save index cache after setting pad {} to Confirmed: {}. Proceeding anyway.",
                                    pad_info.address, e
                                );
                            }
                            // --- Emit Callback ---
                            // NOTE: Locking callback_arc here
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
                            // --- Success ---
                            return Ok(pad_info.chunk_index);
                        }
                        Err(e) => {
                            error!(
                                "Failed to update pad status to Confirmed for {}: {}",
                                pad_info.address, e
                            );
                            // Return immediately on index error
                            return Err(DataError::Index(e));
                        }
                    }
                } // end if counter_check_passed
                  // else: counter check failed, loop will continue after delay
            }
            Err(fetch_err) => {
                // Check if the error is a retriable NotEnoughCopies
                let is_not_enough_copies = if let StorageError::Network(net_err) = &fetch_err {
                    if let crate::network::error::NetworkError::InternalError(msg) = net_err {
                        // Check the underlying SDK error message string
                        msg.contains("NotEnoughCopies")
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_not_enough_copies {
                    // Treat NotEnoughCopies as a transient error, similar to failed counter check
                    trace!(
                        "Attempt {}: Transient fetch error for pad {}: {} Retrying...",
                        attempt + 1,
                        pad_info.address,
                        fetch_err
                    );
                    // Store the error in case the loop times out
                    last_error = Some(DataError::Storage(fetch_err.into()));
                    // DO NOT return; allow the loop to sleep and retry
                } else {
                    // For other storage errors, fail immediately
                    error!(
                        "Confirmation failed for pad {}: Non-retriable storage error during fetch: {}",
                        pad_info.address, fetch_err
                    );
                    // Return the non-retriable error
                    return Err(DataError::Storage(fetch_err.into()));
                }
            }
        }

        // If counter check failed, size check failed, OR a retriable fetch error occurred, wait before next attempt
        if attempt < CONFIRMATION_RETRY_LIMIT - 1 {
            sleep(CONFIRMATION_RETRY_DELAY).await;
        }
    } // --- End Confirmation Retry Loop ---

    // If loop finished without success, return the last stored error or a timeout error
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

/// Executes the concurrent write and confirmation tasks for a store operation.
async fn execute_write_confirm_tasks(
    deps: DataManagerDependencies, // Clone the whole struct for simplicity here
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

    // Clone needed dependencies for async blocks/tasks
    // Note: Cloning the whole `deps` struct might be easier than individual Arcs here
    // if confirm_pad_write also takes the deps struct.
    // Adjust if confirm_pad_write keeps individual Arc args.
    let index_manager = Arc::clone(&deps.index_manager);
    let network_adapter = Arc::clone(&deps.network_adapter);
    let storage_manager = Arc::clone(&deps.storage_manager);
    let pad_lifecycle_manager = Arc::clone(&deps.pad_lifecycle_manager); // Needed for confirm_pad_write

    // Populate FuturesUnordered from the prepared tasks
    for task_input in tasks_to_run_input {
        let storage_manager_clone = Arc::clone(&storage_manager);
        let secret_key_write = task_input.secret_key.clone();
        let pad_info_write = task_input.pad_info.clone();
        let current_status = pad_info_write.status.clone();
        let chunk_data = task_input.chunk_data;
        let expected_chunk_size = chunk_data.len(); // Capture size here

        pending_writes += 1;
        write_futures.push(async move {
            let write_result = storage_manager_clone
                .write_pad_data(&secret_key_write, &chunk_data, &current_status)
                .await
                .map(|_| ())
                .map_err(|e| DataError::Storage(e.into()));
            // Return the chunk size along with other info
            (
                pad_info_write,
                secret_key_write,
                expected_chunk_size,
                write_result,
            )
        });
    }

    debug!(
        "Processing {} pending writes and {} pending confirms.",
        pending_writes, pending_confirms
    );

    // --- Combined Write & Confirm Loop ---
    while (pending_writes > 0 || pending_confirms > 0) && operation_error.is_none() {
        tokio::select! {
            biased;

            // --- Process Write Task Completion ---
            Some((completed_pad_info, completed_secret_key, completed_chunk_size, write_result)) = write_futures.next(), if pending_writes > 0 => {
                pending_writes -= 1;
                match write_result {
                    Ok(()) => {
                        trace!(
                            "Write successful for chunk {}, pad {}",
                            completed_pad_info.chunk_index,
                            completed_pad_info.address
                        );

                        // Emit PadReserved event
                        // Clone callback_arc for this block
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

                        // Update status to Written
                        // Clone index_manager for this block
                        let index_manager_clone = Arc::clone(&index_manager);
                        let user_key_clone = user_key.clone(); // Clone user_key needed here
                        let completed_pad_address = completed_pad_info.address; // Copy address
                        match index_manager_clone
                            .update_pad_status(&user_key_clone, &completed_pad_address, PadStatus::Written)
                            .await
                        {
                            Ok(_) => {
                                trace!(
                                    "Updated status to Written for pad {}",
                                    completed_pad_address
                                );

                                // Emit ChunkWritten callback
                                // Clone callback_arc again
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

                                // --- Spawn Confirmation Task ---
                                // Clone dependencies needed for confirm_pad_write
                                let index_manager_confirm = Arc::clone(&index_manager);
                                let pad_lifecycle_confirm = Arc::clone(&pad_lifecycle_manager);
                                let storage_manager_confirm = Arc::clone(&storage_manager);
                                let network_adapter_confirm = Arc::clone(&network_adapter);
                                let user_key_clone_confirm = user_key.clone();
                                let callback_arc_clone_confirm = callback_arc.clone();
                                let pad_info_confirm = completed_pad_info.clone();
                                let pad_key_confirm = completed_secret_key.clone(); // Use completed_secret_key
                                let expected_chunk_size_confirm = completed_chunk_size; // Use completed_chunk_size

                                debug!(
                                    "Spawning confirmation task for pad {}",
                                    pad_info_confirm.address
                                );
                                pending_confirms += 1;
                                confirm_futures.push(tokio::spawn(async move {
                                    confirm_pad_write(
                                        index_manager_confirm,
                                        pad_lifecycle_confirm,
                                        storage_manager_confirm,
                                        network_adapter_confirm,
                                        user_key_clone_confirm,
                                        pad_info_confirm,
                                        pad_key_confirm,
                                        expected_chunk_size_confirm, // Use the captured size
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
            } // End Write Completion Handling

            // --- Process Confirmation Task Completion ---
            Some(confirm_join_result) = confirm_futures.next(), if pending_confirms > 0 => {
                pending_confirms -= 1;
                // Handle potential JoinError from tokio::spawn
                match confirm_join_result {
                     Ok(Ok(confirmed_chunk_index)) => {
                        trace!("Confirmation task completed successfully for chunk index {}", confirmed_chunk_index);
                    }
                    Ok(Err(e)) => { // Inner result was Err
                         error!("Confirmation task failed: {}. Halting.", e);
                         if operation_error.is_none() {
                             operation_error = Some(e);
                         }
                         if matches!(operation_error, Some(DataError::OperationCancelled)) {
                            callback_cancelled = true;
                         }
                         continue;
                    }
                    Err(join_err) => { // Task panicked or was cancelled
                        error!("Confirmation task panicked or was cancelled: {}. Halting.", join_err);
                         if operation_error.is_none() {
                            operation_error = Some(DataError::InternalError(format!("Confirmation task failed unexpectedly: {}", join_err)));
                         }
                         continue;
                    }
                }
            } // End Confirmation Completion Handling

            else => {
                // Handled by select! macro, no explicit yield needed normally
            }
        } // End tokio::select!
    } // End combined loop

    // Abort remaining futures if loop exited early due to error or cancellation
    if callback_cancelled || operation_error.is_some() {
        debug!("Aborting remaining write/confirmation tasks due to error or cancellation.");
        while write_futures.next().await.is_some() {}
        while confirm_futures.next().await.is_some() {}
    }

    // Return the final result (Ok or the first error encountered)
    match operation_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
