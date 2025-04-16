use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::events::{
    invoke_get_callback, invoke_put_callback, GetCallback, GetEvent, PutCallback, PutEvent,
};
use crate::index::structure::PadStatus;
use crate::index::{IndexManager, KeyInfo, PadInfo};
use crate::pad_lifecycle::PadLifecycleManager;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::StorageManager;
use autonomi::SecretKey;
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio;
use tokio::sync::Mutex;

// Helper structure to pass down dependencies to operation functions
// Using Arcs for shared ownership across potential concurrent tasks
pub(crate) struct DataManagerDependencies {
    pub index_manager: Arc<dyn IndexManager>,
    pub pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
    pub storage_manager: Arc<dyn StorageManager>,
    // Add network_adapter if needed for existence checks? Yes.
    pub network_adapter: Arc<dyn crate::network::NetworkAdapter>,
}

// Define structure for tasks to be executed concurrently
enum PadTask {
    Write {
        pad_info: PadInfo, // Contains address, chunk_index, initial status (Generated), and origin
        secret_key: SecretKey,
        chunk_data: Vec<u8>,
        is_new_hint: bool, // Hint for storage layer (true=try create, false=try update)
    },
    // Confirm variant was here, removed as unused
}

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

    // 1. Get chunk size and chunk data (do this early)
    let chunk_size = deps.index_manager.get_scratchpad_size().await?;
    let chunks = chunk_data(data_bytes, chunk_size)?;
    let num_chunks = chunks.len();
    debug!("Data chunked into {} pieces.", num_chunks);

    // 2. Check for existing KeyInfo (New vs Resume)
    let existing_key_info_opt = deps.index_manager.get_key_info(&user_key).await?;

    let mut tasks_to_run = Vec::new();
    let _current_key_info: KeyInfo; // Holds the state to be used/updated, prefix with _ as it's assigned but not read

    match existing_key_info_opt {
        // --- Resume Path ---
        Some(mut key_info) => {
            info!(
                "Found existing KeyInfo for key '{}'. Attempting resume.",
                user_key
            );
            if key_info.is_complete {
                info!(
                    "Key '{}' is already marked as complete. Nothing to do.",
                    user_key
                );
                // TODO: Add force flag handling later if needed
                return Ok(()); // Or return KeyAlreadyExists? Ok seems better for idempotent resume.
            }

            // Verify consistency
            if key_info.data_size != data_size {
                error!(
                    "Data size mismatch for key '{}'. Expected {}, got {}. Cannot resume.",
                    user_key, key_info.data_size, data_size
                );
                return Err(DataError::InconsistentState(format!(
                    "Data size mismatch for key '{}'",
                    user_key
                )));
            }
            if key_info.pads.len() != num_chunks {
                error!(
                    "Chunk count mismatch for key '{}'. Index has {}, data has {}. Cannot resume.",
                    user_key,
                    key_info.pads.len(),
                    num_chunks
                );
                return Err(DataError::InconsistentState(format!(
                    "Chunk count mismatch for key '{}'",
                    user_key
                )));
            }

            // --- Calculate Initial Progress for Resume ---
            let mut initial_written_count = 0;
            let mut initial_confirmed_count = 0;
            for pad in &key_info.pads {
                match pad.status {
                    PadStatus::Written => initial_written_count += 1,
                    PadStatus::Confirmed => {
                        initial_written_count += 1; // Confirmed also counts as Written for the first bar
                        initial_confirmed_count += 1;
                    }
                    PadStatus::Generated => {}
                }
            }

            // --- Send Starting Event for Resume (with initial progress) ---
            if !invoke_put_callback(
                &mut *callback_arc.lock().await,
                PutEvent::Starting {
                    total_chunks: num_chunks,
                    initial_written_count,
                    initial_confirmed_count,
                },
            )
            .await
            .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
            {
                warn!("Put operation cancelled by callback during Starting event (resume).");
                return Err(DataError::OperationCancelled);
            }

            // --- Identify pads needing existence check --- START
            let mut checks_needed = Vec::new();
            for pad_info in key_info.pads.iter() {
                if pad_info.status == PadStatus::Generated
                    && pad_info.origin == PadOrigin::Generated
                {
                    checks_needed.push(pad_info.address);
                }
            }
            // --- Identify pads needing existence check --- END

            // --- Perform existence checks concurrently --- START
            let mut existence_results = HashMap::new();
            if !checks_needed.is_empty() {
                debug!(
                    "Performing concurrent existence checks for {} generated pads...",
                    checks_needed.len()
                );
                let mut check_futures = FuturesUnordered::new();
                let network_adapter = Arc::clone(&deps.network_adapter);
                for address in checks_needed {
                    let net_clone = Arc::clone(&network_adapter);
                    check_futures.push(async move {
                        let result = net_clone.check_existence(&address).await;
                        (address, result)
                    });
                }

                while let Some((address, result)) = check_futures.next().await {
                    match result {
                        Ok(exists) => {
                            existence_results.insert(address, exists);
                        }
                        Err(e) => {
                            error!(
                                "Network error checking existence for pad {} during resume preparation: {}. Aborting.",
                                address, e
                            );
                            return Err(DataError::InternalError(format!(
                                "Network existence check failed during resume preparation: {}",
                                e
                            )));
                        }
                    }
                }
                debug!("Concurrent existence checks complete.");
            }
            // --- Perform existence checks concurrently --- END

            // --- Identify tasks based on pad status (using check results) --- START
            debug!("Identifying resume tasks based on pad status...");
            for pad_info in key_info.pads.iter() {
                match pad_info.status {
                    PadStatus::Generated => {
                        if let Some(key_bytes) = key_info.pad_keys.get(&pad_info.address) {
                            let secret_key = SecretKey::from_bytes(
                                key_bytes.clone().try_into().map_err(|_| {
                                    DataError::InternalError(
                                        "Invalid key size in index".to_string(),
                                    )
                                })?,
                            )?;
                            let chunk = chunks.get(pad_info.chunk_index).ok_or_else(|| {
                                DataError::InternalError(format!(
                                    "Chunk index {} out of bounds during resume",
                                    pad_info.chunk_index
                                ))
                            })?;

                            // --- Determine is_new_hint using pre-fetched results --- START
                            let is_new_hint = match pad_info.origin {
                                PadOrigin::FreePool => {
                                    trace!(
                                        "Resume Task: Pad {} from FreePool, setting is_new_hint=false",
                                        pad_info.address
                                    );
                                    false
                                }
                                PadOrigin::Generated => {
                                    let exists = existence_results.get(&pad_info.address).copied().ok_or_else(|| {
                                        // This should not happen if logic is correct
                                        error!("Logic error: Existence result missing for generated pad {}", pad_info.address);
                                        DataError::InternalError("Missing existence check result".to_string())
                                    })?;
                                    let hint = !exists;
                                    trace!(
                                        "Resume Task: Pad {} was Generated, exists: {}. Setting is_new_hint={}",
                                        pad_info.address, exists, hint
                                    );
                                    hint
                                }
                            };
                            // --- Determine is_new_hint using pre-fetched results --- END

                            debug!(
                                "Task: Create write task for chunk {} to pad {} (Origin: {:?}, Hint: {})",
                                pad_info.chunk_index,
                                pad_info.address,
                                pad_info.origin,
                                is_new_hint
                            );

                            tasks_to_run.push(PadTask::Write {
                                pad_info: pad_info.clone(),
                                secret_key: secret_key,
                                chunk_data: chunk.clone(),
                                is_new_hint, // Pass determined hint
                            });
                        } else {
                            warn!(
                                "Missing secret key for pad {} in KeyInfo during resume. Skipping.",
                                pad_info.address
                            );
                            return Err(DataError::InternalError(format!(
                                "Missing key for pad {} during resume",
                                pad_info.address
                            )));
                        }
                    }
                    PadStatus::Written => {
                        debug!(
                            "Pad {} for chunk {} already marked Written. Skipping write task.",
                            pad_info.address, pad_info.chunk_index
                        );
                        // TODO: Add Confirmation task here if explicit confirmation step is needed
                    }
                    PadStatus::Confirmed => {
                        debug!(
                            "Pad {} for chunk {} already marked Confirmed. Skipping.",
                            pad_info.address, pad_info.chunk_index
                        );
                    }
                }
            }
            info!("Prepared {} tasks for resume.", tasks_to_run.len());
            // Update modified timestamp
            key_info.modified = Utc::now();
            deps.index_manager
                .insert_key_info(user_key.clone(), key_info.clone())
                .await?; // Update modified time
            _current_key_info = key_info; // Use the loaded info

            // --- Persist Index Cache after updating modified timestamp for resume ---
            let network_choice = deps.network_adapter.get_network_choice();
            if let Err(e) = deps
                .pad_lifecycle_manager
                .save_index_cache(network_choice)
                .await
            {
                warn!(
                    "Failed to save index cache after resume update: {}. Proceeding anyway.",
                    e
                );
                // Don't necessarily fail the operation, but log the warning.
            } else {
                // Added debug log for successful save
                debug!(
                    "Successfully saved index cache after resume update for key '{}'.",
                    user_key
                );
            }
        }

        // --- New Upload Path ---
        None => {
            info!(
                "No existing KeyInfo found for key '{}'. Starting new upload.",
                user_key
            );

            // Handle empty data case first
            if num_chunks == 0 {
                debug!("Storing empty data for key '{}'", user_key);
                let key_info = KeyInfo {
                    pads: Vec::new(),
                    pad_keys: HashMap::new(),
                    data_size,
                    modified: Utc::now(),
                    is_complete: true, // Empty data is complete
                                       // populated_pads_count removed
                };
                deps.index_manager
                    .insert_key_info(user_key.clone(), key_info)
                    .await?;
                info!("Empty key '{}' created.", user_key);
                if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
                    .await
                    .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
                {
                    return Err(DataError::OperationCancelled);
                }
                return Ok(());
            }

            // Invoke Starting event *before* acquiring pads (with 0 initial progress for new uploads)
            if !invoke_put_callback(
                &mut *callback_arc.lock().await,
                PutEvent::Starting {
                    total_chunks: num_chunks,
                    initial_written_count: 0,
                    initial_confirmed_count: 0,
                },
            )
            .await
            .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
            {
                warn!("Put operation cancelled by callback during Starting event.");
                return Err(DataError::OperationCancelled);
            }

            // Acquire necessary pads (now returns origin and takes callback)
            debug!("Acquiring {} pads...", num_chunks);
            let acquired_pads_with_origin = deps
                .pad_lifecycle_manager
                .acquire_pads(num_chunks, &mut *callback_arc.lock().await)
                .await?;

            debug!(
                "Successfully acquired {} pads.",
                acquired_pads_with_origin.len()
            );

            // Prepare initial KeyInfo and tasks
            let mut initial_pads = Vec::with_capacity(num_chunks);
            let mut initial_pad_keys = HashMap::with_capacity(num_chunks);

            for (i, chunk) in chunks.iter().enumerate() {
                if i >= acquired_pads_with_origin.len() {
                    // Should not happen if acquire_pads worked correctly
                    error!(
                        "Logic error: Acquired pads count ({}) less than chunk index ({})",
                        acquired_pads_with_origin.len(),
                        i
                    );
                    return Err(DataError::InternalError(
                        "Pad acquisition count mismatch".to_string(),
                    ));
                }
                // Destructure the tuple including origin
                let (pad_address, secret_key, pad_origin) = acquired_pads_with_origin[i].clone();

                let pad_info = PadInfo {
                    address: pad_address,
                    chunk_index: i,
                    status: PadStatus::Generated,
                    origin: pad_origin, // Store the origin
                };

                initial_pads.push(pad_info.clone());
                initial_pad_keys.insert(pad_address, secret_key.to_bytes().to_vec());

                debug!(
                    "Task: Write chunk {} to new pad {} (Origin: {:?})",
                    i, pad_address, pad_origin
                );
                tasks_to_run.push(PadTask::Write {
                    pad_info,
                    secret_key,
                    chunk_data: chunk.clone(),
                    is_new_hint: true, // Hint is true for new upload
                });
            }

            let key_info = KeyInfo {
                pads: initial_pads,
                pad_keys: initial_pad_keys,
                data_size,
                modified: Utc::now(),
                is_complete: false, // Not complete yet
            };

            // Insert initial KeyInfo into the index *before* starting writes
            deps.index_manager
                .insert_key_info(user_key.clone(), key_info.clone())
                .await?;
            info!(
                "Initial KeyInfo for '{}' created with {} pads.",
                user_key,
                key_info.pads.len()
            );
            _current_key_info = key_info; // Use the newly created info

            // --- Persist Index Cache after initial KeyInfo insertion ---
            let network_choice = deps.network_adapter.get_network_choice();
            if let Err(e) = deps
                .pad_lifecycle_manager
                .save_index_cache(network_choice)
                .await
            {
                // Corrected variable name in log message
                warn!(
                    "Failed to save initial index cache for key '{}': {}. Proceeding anyway.",
                    user_key, e
                );
                // Don't necessarily fail the operation, but log the warning.
            } else {
                // Added debug log for successful save
                debug!(
                    "Successfully saved initial index cache for key '{}'.",
                    user_key
                );
            }
        }
    }

    // --- Callback for Starting ---
    // Sent after initial setup (new or resume) is done and tasks are prepared
    // MOVED EARLIER, removed redundant call:
    /*
    if !invoke_put_callback(
        &mut callback,
        PutEvent::Starting {
            total_chunks: num_chunks, // Keep total chunks for overall progress
        },
    )
    .await
    .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        info!("Operation cancelled by callback after setup.");
        // No pads need releasing here as the index state is saved.
        return Err(DataError::OperationCancelled);
    }
    */

    // --- Concurrent Processing (Combined Write & Confirm) ---
    debug!(
        "Starting concurrent processing of {} write tasks...",
        tasks_to_run.len()
    );
    let mut write_futures = FuturesUnordered::new();
    let mut confirm_futures = FuturesUnordered::new(); // Added for confirmations

    let mut operation_error: Option<DataError> = None;
    let mut callback_cancelled = false;
    let mut pending_writes = 0;
    let mut pending_confirms = 0; // Added counter

    // Clone needed dependencies for async blocks
    let index_manager = Arc::clone(&deps.index_manager);
    let network_adapter = Arc::clone(&deps.network_adapter);
    let storage_manager = Arc::clone(&deps.storage_manager);
    let user_key_clone = user_key.clone(); // Clone user_key for use in tasks

    // Populate FuturesUnordered from the prepared tasks
    for task in tasks_to_run {
        match task {
            PadTask::Write {
                pad_info,
                secret_key,
                chunk_data,
                is_new_hint,
            } => {
                // Clone for the write task future
                let storage_manager_clone = Arc::clone(&storage_manager);
                // Remove clone for network_adapter - no longer needed here
                // let network_adapter_clone = Arc::clone(&network_adapter);

                pending_writes += 1;
                write_futures.push(async move {
                    // Remove the complex existence check logic from here
                    // The hint is now determined *before* task creation.
                    let write_result = storage_manager_clone
                        .write_pad_data(&secret_key, &chunk_data, is_new_hint) // Use the hint from the task
                        .await
                        .map(|_| ()) // Discard the address on success, keep unit type
                        .map_err(|e| DataError::Storage(e.into())); // Map StorageError to DataError

                    // Return original pad_info along with the result
                    (pad_info, write_result)
                });
            }
        }
    }

    debug!(
        "Processing {} pending writes and {} pending confirms.",
        pending_writes, pending_confirms
    );

    // --- Combined Write & Confirm Loop ---
    while (pending_writes > 0 || pending_confirms > 0) && operation_error.is_none() {
        tokio::select! {
            // Biased select to prioritize processing completions over potential new confirmations
            biased;

            // --- Process Write Task Completion ---
            Some((pad_info, result)) = write_futures.next(), if pending_writes > 0 => {
                pending_writes -= 1;
                match result {
                    Ok(()) => {
                        trace!("Write successful for chunk {}, pad {}", pad_info.chunk_index, pad_info.address);

                        // *** Emit PadReserved event HERE upon successful write ***
                        if !invoke_put_callback(
                            &mut *callback_arc.lock().await,
                            PutEvent::PadReserved { count: 1 }
                        )
                        .await
                        .map_err(|e| DataError::InternalError(format!("Callback invocation failed for PadReserved: {}", e)))?
                        {
                             error!("Store operation cancelled by callback after pad reservation (write success).");
                             operation_error = Some(DataError::OperationCancelled);
                             callback_cancelled = true;
                             continue; // Let select break on error check
                        }

                        // Update status to Written
                        match index_manager
                            .update_pad_status(&user_key_clone, &pad_info.address, PadStatus::Written)
                            .await
                        {
                            Ok(_) => {
                                trace!("Updated status to Written for pad {}", pad_info.address);

                                // Emit ChunkWritten callback
                                 if !invoke_put_callback(
                                     &mut *callback_arc.lock().await,
                                     PutEvent::ChunkWritten { chunk_index: pad_info.chunk_index }
                                     )
                                     .await
                                     .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
                                 {
                                     error!("Store operation cancelled by callback after chunk write.");
                                     operation_error = Some(DataError::OperationCancelled);
                                     callback_cancelled = true;
                                     continue; // Let select break on error check
                                 }

                                // --- Spawn Confirmation Task ---
                                let network_adapter_clone = Arc::clone(&network_adapter);
                                let index_manager_clone = Arc::clone(&index_manager);
                                let user_key_inner_clone = user_key_clone.clone();
                                let callback_clone = Arc::clone(&callback_arc); // Clone the Arc
                                let pad_lifecycle_manager_clone = Arc::clone(&deps.pad_lifecycle_manager); // Clone for use in confirm task
                                let confirm_pad_info = PadInfo { // Create info with the correct address
                                    address: pad_info.address,
                                    chunk_index: pad_info.chunk_index,
                                    status: PadStatus::Written, // Status is now Written
                                    origin: pad_info.origin, // *** Propagate origin from outer scope ***
                                };

                                debug!("Spawning confirmation task for pad {}", pad_info.address);
                                pending_confirms += 1;
                                confirm_futures.push(async move {
                                    let confirmation_result = network_adapter_clone
                                        .check_existence(&confirm_pad_info.address)
                                        .await;

                                    match confirmation_result {
                                        Ok(exists) => {
                                            if exists {
                                                trace!(
                                                    "Confirmation check successful for chunk {}, pad {}",
                                                    confirm_pad_info.chunk_index, confirm_pad_info.address
                                                );
                                                // Update status to Confirmed
                                                match index_manager_clone
                                                    .update_pad_status(&user_key_inner_clone, &confirm_pad_info.address, PadStatus::Confirmed)
                                                    .await {
                                                        Ok(_) => {
                                                            trace!("Updated status to Confirmed for pad {}", confirm_pad_info.address);

                                                            // --- Persist Index Cache after PadStatus::Confirmed update ---
                                                            let network_choice = network_adapter_clone.get_network_choice(); // Use correct clone
                                                            if let Err(e) = pad_lifecycle_manager_clone.save_index_cache(network_choice).await { // Use the cloned manager
                                                                warn!("Failed to save index cache after setting pad {} to Confirmed: {}. Proceeding anyway.", confirm_pad_info.address, e);
                                                            }

                                                            // Emit ChunkConfirmed callback
                                                            if !invoke_put_callback(
                                                                &mut *callback_clone.lock().await,
                                                                PutEvent::ChunkConfirmed {
                                                                    chunk_index: confirm_pad_info.chunk_index,
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
                                                                // Indicate cancellation via error
                                                                return Err(DataError::OperationCancelled);
                                                            }
                                                            Ok(confirm_pad_info.chunk_index) // Indicate success
                                                        }
                                                        Err(e) => {
                                                            error!("Failed to update pad status to Confirmed for {}: {}", confirm_pad_info.address, e);
                                                            Err(DataError::Index(e)) // Return index error
                                                        }
                                                    }
                                            } else {
                                                // Confirmation failed (pad doesn't exist after write?)
                                                error!("Confirmation check failed for pad {}: Pad not found after write.", confirm_pad_info.address);
                                                Err(DataError::InconsistentState(format!(
                                                    "Pad {} not found during confirmation",
                                                    confirm_pad_info.address
                                                )))
                                            }
                                        }
                                        Err(e) => {
                                            // Network error during confirmation check
                                            error!("Network error during confirmation for pad {}: {}", confirm_pad_info.address, e);
                                            Err(DataError::InternalError(format!("Network confirmation failed: {}", e)))
                                        }
                                    }
                                }); // End of confirm_futures.push async block

                            }
                            Err(e) => {
                                // Handle IndexError from update_pad_status
                                error!(
                                    "Failed to update pad status to Written for {}: {}. Halting.",
                                    pad_info.address, e
                                );
                                operation_error = Some(DataError::Index(e)); // Correct: Wrap IndexError
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        // Handle DataError from the write future itself
                        error!(
                            "Failed to write chunk {} to pad {}: {}. Halting.",
                            pad_info.chunk_index, pad_info.address, e
                        );
                        if operation_error.is_none() {
                            operation_error = Some(e); // Correct: Assign DataError directly
                        }
                        continue;
                    }
                }
            } // End Write Completion Handling

            // --- Process Confirmation Task Completion ---
            Some(confirm_result) = confirm_futures.next(), if pending_confirms > 0 => {
                pending_confirms -= 1;
                match confirm_result {
                    Ok(confirmed_chunk_index) => {
                        trace!("Confirmation task completed successfully for chunk index {}", confirmed_chunk_index);
                        // Status update and callback emission happened inside the task's future
                    }
                    Err(e) => {
                         error!("Confirmation task failed: {}. Halting.", e);
                         if operation_error.is_none() { // Prevent overwriting earlier error
                             operation_error = Some(e); // Use the error returned by the confirmation task
                         }
                         // If the error was OperationCancelled from the callback, set the flag
                         if matches!(operation_error, Some(DataError::OperationCancelled)) {
                            callback_cancelled = true;
                         }
                         continue; // Let select break on error check
                    }
                }
            } // End Confirmation Completion Handling

            // Default branch if no futures are ready - avoids busy-waiting
            else => {
                // Optional: yield briefly if needed, though select! handles this
                // tokio::task::yield_now().await;
            }

        } // End tokio::select!
    } // End combined loop

    // Abort remaining futures if loop exited early due to error or cancellation
    if callback_cancelled || operation_error.is_some() {
        debug!("Aborting remaining write/confirmation tasks due to error or cancellation.");
        while write_futures.next().await.is_some() {}
        while confirm_futures.next().await.is_some() {}
    }

    // --- Final Check and Completion ---
    // If an error occurred or was cancelled, return the error
    if let Some(err) = operation_error {
        warn!("Store operation for key '{}' failed: {}", user_key, err);
        return Err(err);
    }

    // Re-fetch the latest KeyInfo to check final status
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
        // Consider if this state requires returning an error or if incomplete is acceptable.
        // For now, we proceed to the Complete callback but don't mark as complete.
    }

    // Send Complete callback
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
        // Don't return error here, just log, as the operation itself is done.
        warn!("Operation cancelled by callback after completion.");
    }

    Ok(())
}

// --- Fetch Operation ---

pub(crate) async fn fetch_op(
    deps: &DataManagerDependencies,
    user_key: &str,
    mut callback: Option<GetCallback>,
) -> Result<Vec<u8>, DataError> {
    info!("DataOps: Starting fetch operation for key '{}'", user_key);

    // 1. Get KeyInfo and MasterIndex copy
    let key_info = deps
        .index_manager
        .get_key_info(user_key)
        .await?
        .ok_or_else(|| DataError::KeyNotFound(user_key.to_string()))?;

    // Fetch the current index for validation and pad info
    // Use underscore as index_copy is not directly used after this.
    let _index_copy = deps.index_manager.get_index_copy().await?; // Fetch once

    if !key_info.is_complete {
        // Handle incomplete data - return error or partial data? Error for now.
        warn!("Attempting to fetch incomplete data for key '{}'", user_key);
        return Err(DataError::InternalError(format!(
            "Data for key '{}' is marked as incomplete",
            user_key
        )));
    }

    let num_chunks = key_info.pads.len();
    debug!("Found {} chunks for key '{}'", num_chunks, user_key);

    if !invoke_get_callback(
        &mut callback,
        GetEvent::Starting {
            total_chunks: num_chunks,
        },
    )
    .await
    .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        return Err(DataError::OperationCancelled);
    }

    // Handle empty data case
    if num_chunks == 0 {
        debug!("Fetching empty data for key '{}'", user_key);
        if key_info.data_size != 0 {
            warn!(
                "Index inconsistency: 0 pads but data_size is {}",
                key_info.data_size
            );
            // Return empty vec anyway? Or error? Let's return empty vec.
        }
        if !invoke_get_callback(&mut callback, GetEvent::Complete)
            .await
            .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
        {
            return Err(DataError::OperationCancelled);
        }
        return Ok(Vec::new());
    }

    // 2. Fetch chunks concurrently
    let mut fetch_futures = FuturesUnordered::new();
    let mut sorted_pads = key_info.pads.clone(); // Clone to avoid borrow issues later
    sorted_pads.sort_by_key(|p| p.chunk_index);

    let storage_manager = Arc::clone(&deps.storage_manager);
    for pad_info in sorted_pads.iter() {
        let sm_clone = Arc::clone(&storage_manager);
        let address = pad_info.address;
        let index = pad_info.chunk_index;
        fetch_futures.push(async move {
            let result = sm_clone.read_pad_scratchpad(&address).await;
            (index, address, result) // Return index, address, and Result<Scratchpad, StorageError>
        });
    }

    // Collect fetched *and decrypted* chunks
    let mut fetched_decrypted_chunks: Vec<Option<Vec<u8>>> = vec![None; num_chunks];
    let mut fetched_count = 0;

    while let Some((chunk_index, pad_address, result)) = fetch_futures.next().await {
        match result {
            Ok(scratchpad) => {
                trace!(
                    "Successfully fetched scratchpad for chunk {} from pad {}",
                    chunk_index,
                    pad_address
                );

                // Find the pad key from the KeyInfo.pad_keys map
                let key_bytes_vec = key_info.pad_keys.get(&pad_address).ok_or_else(|| {
                    error!("Secret key for pad {} not found in KeyInfo", pad_address);
                    DataError::InternalError(format!("Pad key missing for {}", pad_address))
                })?;

                let pad_secret_key = {
                    let key_array: [u8; 32] =
                        key_bytes_vec.as_slice().try_into().map_err(|_| {
                            error!(
                                "Secret key for pad {} has incorrect length (expected 32): {}",
                                pad_address,
                                key_bytes_vec.len()
                            );
                            DataError::InternalError(format!(
                                "Invalid key length for {}",
                                pad_address
                            ))
                        })?;
                    SecretKey::from_bytes(key_array).map_err(|e| {
                        error!(
                            "Failed to deserialize secret key for pad {}: {}",
                            pad_address, e
                        );
                        DataError::InternalError(format!(
                            "Pad key deserialization failed for {}",
                            pad_address
                        ))
                    })?
                };

                // Decrypt the data using scratchpad.decrypt_data() - NOW INSIDE THE BLOCK
                let decrypted_data = scratchpad.decrypt_data(&pad_secret_key).map_err(|e| {
                    error!(
                        "Failed to decrypt chunk {} from pad {}: {}",
                        chunk_index, pad_address, e
                    );
                    // Use InternalError for decryption failures
                    DataError::InternalError(format!(
                        "Chunk decryption failed for pad {}",
                        pad_address
                    ))
                })?;
                trace!("Decrypted chunk {} successfully", chunk_index);

                if chunk_index < fetched_decrypted_chunks.len() {
                    fetched_decrypted_chunks[chunk_index] = Some(decrypted_data.to_vec());
                    fetched_count += 1;
                    if !invoke_get_callback(&mut callback, GetEvent::ChunkFetched { chunk_index })
                        .await
                        .map_err(|e| {
                            DataError::InternalError(format!("Callback invocation failed: {}", e))
                        })?
                    {
                        error!("Fetch operation cancelled by callback during chunk fetching.");
                        return Err(DataError::OperationCancelled);
                    }
                } else {
                    error!(
                        "Invalid chunk index {} returned during fetch (max expected {})",
                        chunk_index,
                        num_chunks - 1
                    );
                    return Err(DataError::InternalError(format!(
                        "Invalid chunk index {} encountered",
                        chunk_index
                    )));
                }
            }
            Err(e) => {
                error!(
                    "Failed to fetch scratchpad for chunk {}: {}",
                    chunk_index, e
                );
                return Err(DataError::Storage(e.into()));
            }
        }
    }

    if fetched_count != num_chunks {
        error!(
            "Fetched {} chunks, but expected {}",
            fetched_count, num_chunks
        );
        // This implies some futures didn't complete or returned invalid indices, should not happen without error above.
        return Err(DataError::InternalError(
            "Mismatch between expected and fetched chunk count".to_string(),
        ));
    }

    debug!("All {} chunks fetched and decrypted.", num_chunks);

    // 3. Reassemble *decrypted* data
    if !invoke_get_callback(&mut callback, GetEvent::Reassembling)
        .await
        .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        return Err(DataError::OperationCancelled);
    }
    let reassembled_data = reassemble_data(fetched_decrypted_chunks, key_info.data_size)?;
    debug!("Decrypted data reassembled successfully.");

    if !invoke_get_callback(&mut callback, GetEvent::Complete)
        .await
        .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        return Err(DataError::OperationCancelled);
    }

    info!("DataOps: Fetch operation complete for key '{}'", user_key);
    Ok(reassembled_data) // Return the reassembled decrypted data
}

// --- Remove Operation ---

pub(crate) async fn remove_op(
    deps: &DataManagerDependencies,
    user_key: &str,
) -> Result<(), DataError> {
    info!("DataOps: Starting remove operation for key '{}'", user_key);

    // 1. Remove key info from index, getting the old info
    let removed_info = deps.index_manager.remove_key_info(user_key).await?;

    match removed_info {
        Some(_key_info) => {
            // Use _key_info as it's not needed after removal
            debug!("Removed key info for '{}' from index.", user_key);
            // The call to index_manager.remove_key_info above handles
            // sorting the pads associated with the removed key into
            // either the free_pads or pending_verification_pads list
            // based on their status. No explicit release call needed here.

            info!("DataOps: Remove operation complete for key '{}'.", user_key);
            Ok(())
        }
        None => {
            warn!("Attempted to remove non-existent key '{}'", user_key);
            // Return Ok or KeyNotFound? Original returned Ok. Let's stick with that.
            Ok(())
            // Err(DataError::KeyNotFound(user_key.to_string()))
        }
    }
}

/* --> START COMMENTING OUT update_op
pub(crate) async fn update_op(
    deps: &DataManagerDependencies,
    user_key: String,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!(
        "DataOps: Starting update operation for key '{}' (via remove+store)",
        user_key
    );

    // 1. Remove the existing key and its associated data/pads.
    // remove_op handles KeyNotFound gracefully, returning Ok(()).
    debug!("UpdateOp: Removing existing key '{}' first...", user_key);
    remove_op(deps, &user_key).await?;
    debug!(
        "UpdateOp: Existing key '{}' removed (or did not exist).",
        user_key
    );

    // 2. Store the new data under the same key.
    // store_op will acquire new pads and write the data.
    debug!("UpdateOp: Storing new data for key '{}'...", user_key);
    // Pass ownership of user_key and callback to store_op
    store_op(deps, user_key, data_bytes, callback).await
}
*/
// <-- END COMMENTING OUT update_op

// --- Helper: Checksum Calculation --- (assuming this is still needed elsewhere or can be removed later)
// ... existing code ...
