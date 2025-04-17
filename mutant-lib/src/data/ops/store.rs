// Store operation logic

use crate::data::chunking::{chunk_data, reassemble_data}; // reassemble_data not used here, but keeping for potential future needs
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
use tokio::time::sleep;

use super::common::{DataManagerDependencies, CONFIRMATION_RETRY_DELAY, CONFIRMATION_RETRY_LIMIT};

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
            // --- List to track pads needing replacement --- START
            let mut pads_needing_replacement = Vec::new();
            // --- List to track pads needing replacement --- END

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
                            // --- Handle NotEnoughCopies specifically --- START
                            let error_string = e.to_string(); // Avoid multiple calls to to_string()
                            if error_string.contains("NotEnoughCopies") {
                                warn!(
                                    "Network existence check for pad {} failed with NotEnoughCopies during resume. Marking for replacement.",
                                    address
                                );
                                // Mark for replacement after the loop
                                pads_needing_replacement.push(address);
                                // Treat as non-existent for initial task scan, will be replaced shortly
                                existence_results.insert(address, false);
                            } else {
                                // --- Handle other errors as before --- START
                                error!(
                                    "Network error checking existence for pad {} during resume preparation: {}. Aborting.",
                                    address, e
                                );
                                // Store error and break loop for consistency?
                                // Or return immediately as this is not a recoverable replacement scenario?
                                // Let's return immediately for non-NotEnoughCopies errors.
                                return Err(DataError::InternalError(format!(
                                    "Network existence check failed during resume preparation: {}",
                                    e
                                )));
                                // --- Handle other errors as before --- END
                            }
                            // --- Handle NotEnoughCopies specifically --- END
                        }
                    }
                }
                debug!("Concurrent existence checks complete.");
            }
            // --- Perform existence checks concurrently --- END

            // --- Replace pads that failed existence check --- START
            if !pads_needing_replacement.is_empty() {
                info!(
                    "Replacing {} pads that failed existence check...",
                    pads_needing_replacement.len()
                );
                // Get mutable access to key_info again
                let mut key_info_mut = key_info;
                key_info_mut.is_complete = false; // Mark incomplete due to replacement

                for old_address in pads_needing_replacement {
                    debug!("Acquiring replacement for pad {}", old_address);
                    // Use ? to propagate DataError from acquire_pads
                    let acquired = deps
                        .pad_lifecycle_manager
                        .acquire_pads(1, &mut *callback_arc.lock().await)
                        .await?;

                    // Handle the success case (guaranteed Ok due to ?)
                    if acquired.len() == 1 {
                        let (new_address, new_secret_key, new_origin) =
                            acquired.into_iter().next().unwrap(); // Safe unwrap
                        info!(
                            "Acquired replacement pad {} for original pad {}",
                            new_address, old_address
                        );

                        if let Some(pad_info_mut) = key_info_mut
                            .pads
                            .iter_mut()
                            .find(|p| p.address == old_address)
                        {
                            // --- Add old pad to pending list --- START
                            if let Some(old_key_bytes) =
                                key_info_mut.pad_keys.get(&old_address).cloned()
                            {
                                trace!(
                                    "Adding abandoned pad {} to pending verification list",
                                    old_address
                                );
                                if let Err(e) = deps
                                    .index_manager
                                    .add_pending_pads(vec![(old_address, old_key_bytes)])
                                    .await
                                {
                                    warn!("Failed to add abandoned pad {} to pending list: {}. Proceeding anyway.", old_address, e);
                                }
                            } else {
                                warn!("Could not find key for abandoned pad {} to add to pending list.", old_address);
                            }
                            // --- Add old pad to pending list --- END

                            // Update KeyInfo to use the new pad
                            pad_info_mut.address = new_address;
                            pad_info_mut.origin = new_origin;
                            pad_info_mut.needs_reverification = false; // New pad doesn't need reverification yet
                            pad_info_mut.status = PadStatus::Generated; // Ensure status is Generated for the new pad

                            // Update pad keys map
                            key_info_mut.pad_keys.remove(&old_address); // Remove old key entry
                            key_info_mut
                                .pad_keys
                                .insert(new_address, new_secret_key.to_bytes().to_vec());
                        // Insert new key
                        } else {
                            error!(
                                "Logic error: Could not find PadInfo for address {} to replace after existence check failure.",
                                old_address
                            );
                            return Err(DataError::InternalError(format!(
                                "Failed to find pad {} for replacement",
                                old_address
                            )));
                        }
                    } else {
                        // Handles len != 1 case
                        error!("Pad acquisition returned unexpected number of pads ({}) when acquiring replacement.", acquired.len());
                        return Err(DataError::InternalError(
                            "Unexpected number of pads returned during replacement".to_string(),
                        ));
                    }
                }
                // --- Persist Index Cache Immediately After ALL Replacements --- START
                let network_choice = deps.network_adapter.get_network_choice();
                if let Err(cache_err) = deps
                    .pad_lifecycle_manager
                    .save_index_cache(network_choice)
                    .await
                {
                    warn!(
                        "Failed to save index cache after replacing pads: {}. Proceeding anyway.",
                        cache_err
                    );
                } else {
                    debug!("Index cache saved after replacing pads.");
                }
                // --- Persist Index Cache Immediately After ALL Replacements --- END

                // Assign replaced key_info back
                key_info = key_info_mut;
            }
            // --- Replace pads that failed existence check --- END

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
                                PadOrigin::FreePool { initial_counter: _ } => {
                                    trace!(
                                        "Resume Task: Pad {} from FreePool, setting is_new_hint=false",
                                        pad_info.address
                                    );
                                    false // Always update free pool pads
                                }
                                PadOrigin::Generated => {
                                    // Check if we have an existence result for this specific pad address
                                    if let Some(exists) = existence_results.get(&pad_info.address) {
                                        // We checked this one earlier and it didn't fail with NotEnoughCopies
                                        let hint = !*exists;
                                        trace!(
                                            "Resume Task: Pad {} (Generated) has existence result: {}. Setting is_new_hint={}",
                                            pad_info.address, exists, hint
                                        );
                                        hint
                                    } else {
                                        // This pad must be a replacement acquired due to NotEnoughCopies on the original,
                                        // or the check loop was skipped. Assume new.
                                        trace!(
                                            "Resume Task: Pad {} (Generated) has no existence result, assuming new (hint=true)",
                                            pad_info.address
                                        );
                                        true
                                    }
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
                    origin: pad_origin,          // Store the origin
                    needs_reverification: false, // Initialize new field
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
                let secret_key_write = secret_key.clone(); // Clone key for write task
                let pad_info_write = pad_info.clone(); // Clone pad_info for write task

                pending_writes += 1;
                write_futures.push(async move {
                    let write_result = storage_manager_clone
                        .write_pad_data(&secret_key_write, &chunk_data, is_new_hint)
                        .await
                        .map(|_| ()) // Discard the address on success, keep unit type
                        .map_err(|e| DataError::Storage(e.into())); // Map StorageError to DataError

                    // Return original pad_info, the key used, and the result
                    (pad_info_write, secret_key_write, write_result)
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
            Some((completed_pad_info, completed_secret_key, write_result)) = write_futures.next(), if pending_writes > 0 => {
                pending_writes -= 1;
                match write_result {
                    Ok(()) => {
                        trace!("Write successful for chunk {}, pad {}", completed_pad_info.chunk_index, completed_pad_info.address);

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
                            .update_pad_status(&user_key_clone, &completed_pad_info.address, PadStatus::Written)
                            .await
                        {
                            Ok(_) => {
                                trace!("Updated status to Written for pad {}", completed_pad_info.address);

                                // Emit ChunkWritten callback
                                 if !invoke_put_callback(
                                     &mut *callback_arc.lock().await,
                                     PutEvent::ChunkWritten { chunk_index: completed_pad_info.chunk_index }
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
                                // Clone managers and other needed variables
                                let storage_manager_clone = Arc::clone(&storage_manager);
                                let index_manager_clone = Arc::clone(&index_manager);
                                let user_key_inner_clone = user_key_clone.clone();
                                let callback_clone = Arc::clone(&callback_arc);
                                let pad_lifecycle_manager_clone = Arc::clone(&deps.pad_lifecycle_manager);
                                let network_adapter_clone = Arc::clone(&network_adapter);
                                // Use the pad_info and key received from the completed write task
                                let confirm_pad_info = completed_pad_info.clone(); // Use completed_pad_info
                                let confirm_secret_key = completed_secret_key.clone(); // Use completed_secret_key

                                debug!("Spawning confirmation task (fetch & decrypt) for pad {}", confirm_pad_info.address);
                                pending_confirms += 1;
                                confirm_futures.push(async move {
                                    // --- Confirmation Retry Loop --- START
                                    let mut last_error: Option<DataError> = None;
                                    for attempt in 0..CONFIRMATION_RETRY_LIMIT {
                                        trace!("Confirmation attempt {} for pad {}", attempt + 1, confirm_pad_info.address);
                                        // Fetch the scratchpad using the cloned storage manager
                                        let fetch_result = storage_manager_clone
                                            .read_pad_scratchpad(&confirm_pad_info.address)
                                            .await;

                                        match fetch_result {
                                            Ok(scratchpad) => {
                                                // Directly get the counter and perform the check
                                                let final_counter = scratchpad.counter();
                                                trace!(
                                                    "Attempt {}: Fetched scratchpad for pad {}. Final counter: {}",
                                                    attempt + 1, confirm_pad_info.address, final_counter
                                                );

                                                // Check counter increment for FreePool pads
                                                let counter_check_passed = match confirm_pad_info.origin {
                                                    PadOrigin::FreePool { initial_counter } => {
                                                        if final_counter > initial_counter {
                                                            trace!("Attempt {}: Counter check passed for FreePool pad {}.", attempt + 1, confirm_pad_info.address);
                                                            true // Counter incremented
                                                        } else {
                                                            trace!(
                                                                "Attempt {}: Counter check failed for pad {}. Initial: {}, Final: {}. Retrying...",
                                                                attempt + 1, confirm_pad_info.address, initial_counter, final_counter
                                                            );
                                                            // Store error in case loop finishes
                                                            last_error = Some(DataError::InconsistentState(format!(
                                                                "Counter check failed after retries for pad {} ({} -> {})",
                                                                confirm_pad_info.address,
                                                                initial_counter,
                                                                final_counter
                                                            )));
                                                            false // Counter did not increment yet
                                                        }
                                                    }
                                                    PadOrigin::Generated => {
                                                        trace!("Attempt {}: Skipping counter check for Generated pad {}.", attempt + 1, confirm_pad_info.address);
                                                        true // No check needed for generated pads
                                                    }
                                                };

                                                if counter_check_passed {
                                                    // Counter check passed (or skipped), proceed to update status
                                                    match index_manager_clone
                                                        .update_pad_status(&user_key_inner_clone, &confirm_pad_info.address, PadStatus::Confirmed)
                                                        .await {
                                                            Ok(_) => {
                                                                trace!("Updated status to Confirmed for pad {}", confirm_pad_info.address);
                                                                // --- Persist Index Cache ---
                                                                let network_choice = network_adapter_clone.get_network_choice();
                                                                if let Err(e) = pad_lifecycle_manager_clone.save_index_cache(network_choice).await {
                                                                    warn!("Failed to save index cache after setting pad {} to Confirmed: {}. Proceeding anyway.", confirm_pad_info.address, e);
                                                                }
                                                                // --- Emit Callback ---
                                                                if !invoke_put_callback(
                                                                    &mut *callback_clone.lock().await,
                                                                    PutEvent::ChunkConfirmed {
                                                                        chunk_index: confirm_pad_info.chunk_index,
                                                                    },
                                                                )
                                                                .await
                                                                .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))? {
                                                                    error!("Store operation cancelled by callback after chunk confirmation.");
                                                                    return Err(DataError::OperationCancelled);
                                                                }
                                                                // --- Success ---
                                                                return Ok(confirm_pad_info.chunk_index);
                                                            }
                                                            Err(e) => {
                                                                error!("Failed to update pad status to Confirmed for {}: {}", confirm_pad_info.address, e);
                                                                // Return immediately on index error
                                                                return Err(DataError::Index(e));
                                                            }
                                                    }
                                                } // end if counter_check_passed
                                                // else: counter check failed, loop will continue after delay
                                            }
                                            Err(fetch_err) => {
                                                // Fetch failed (StorageError) - DO NOT RETRY
                                                error!(
                                                    "Confirmation failed for pad {}: Storage error during fetch: {}",
                                                    confirm_pad_info.address, fetch_err
                                                );
                                                // Ensure error is wrapped correctly
                                                return Err(DataError::Storage(fetch_err.into())); // Keep this error handling
                                            }
                                        }

                                        // If counter check failed, wait before next attempt
                                        if attempt < CONFIRMATION_RETRY_LIMIT - 1 {
                                            sleep(CONFIRMATION_RETRY_DELAY).await;
                                        }
                                    } // --- End Confirmation Retry Loop ---

                                    // If loop finished without success, return the last stored error or a timeout error
                                    error!("Confirmation failed for pad {} after {} attempts.", confirm_pad_info.address, CONFIRMATION_RETRY_LIMIT);
                                    Err(last_error.unwrap_or_else(|| DataError::InternalError(format!(
                                        "Confirmation timed out for pad {}", confirm_pad_info.address
                                    ))))
                                }); // End of confirm_futures.push async block

                            }
                            Err(e) => {
                                // Handle IndexError from update_pad_status
                                error!(
                                    "Failed to update pad status to Written for {}: {}. Halting.",
                                    completed_pad_info.address, e
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
                            completed_pad_info.chunk_index, completed_pad_info.address, e
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
