use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::events::{
    invoke_get_callback, invoke_put_callback, GetCallback, GetEvent, PutCallback, PutEvent,
};
use crate::index::structure::PadStatus;
use crate::index::{IndexManager, KeyInfo, PadInfo};
use crate::pad_lifecycle::PadLifecycleManager;
use crate::storage::StorageManager;
use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use std::collections::HashMap;
use std::sync::Arc;

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
        pad_info: PadInfo, // Contains address, chunk_index, initial status (Generated)
        secret_key: SecretKey,
        chunk_data: Vec<u8>,
    },
    // Add Confirm task later if needed
}

// --- Store Operation ---

pub(crate) async fn store_op(
    deps: &DataManagerDependencies,
    user_key: String, // Take ownership
    data_bytes: &[u8],
    mut callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!("DataOps: Starting store operation for key '{}'", user_key);
    let data_size = data_bytes.len();

    // 1. Get chunk size and chunk data (do this early)
    let chunk_size = deps.index_manager.get_scratchpad_size().await?;
    let chunks = chunk_data(data_bytes, chunk_size)?;
    let num_chunks = chunks.len();
    debug!("Data chunked into {} pieces.", num_chunks);

    // 2. Check for existing KeyInfo (New vs Resume)
    let existing_key_info = deps.index_manager.get_key_info(&user_key).await?;

    let mut tasks_to_run = Vec::new();
    let mut current_key_info: KeyInfo; // Holds the state to be used/updated

    match existing_key_info {
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

            // Identify tasks based on pad status
            debug!("Identifying resume tasks based on pad status...");
            for pad_info in &key_info.pads {
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
                            debug!(
                                "Task: Write chunk {} to pad {}",
                                pad_info.chunk_index, pad_info.address
                            );
                            tasks_to_run.push(PadTask::Write {
                                pad_info: pad_info.clone(), // Clone needed info
                                secret_key,
                                chunk_data: chunk.clone(), // Clone chunk data
                            });
                        } else {
                            warn!(
                                "Missing secret key for pad {} in KeyInfo during resume. Skipping.",
                                pad_info.address
                            );
                            // This shouldn't happen with consistent index, treat as error?
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
            current_key_info = key_info; // Use the loaded info
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
                if !invoke_put_callback(&mut callback, PutEvent::Complete)
                    .await
                    .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
                {
                    return Err(DataError::OperationCancelled);
                }
                return Ok(());
            }

            // Acquire necessary pads
            debug!("Acquiring {} pads...", num_chunks);
            let acquired_pads = deps.pad_lifecycle_manager.acquire_pads(num_chunks).await?;
            // Error handling for insufficient pads is handled by acquire_pads returning Err

            debug!("Successfully acquired {} pads.", acquired_pads.len());

            // Prepare initial KeyInfo and tasks
            let mut initial_pads = Vec::with_capacity(num_chunks);
            let mut initial_pad_keys = HashMap::with_capacity(num_chunks);

            for (i, chunk) in chunks.iter().enumerate() {
                if i >= acquired_pads.len() {
                    // Should not happen if acquire_pads worked correctly
                    error!(
                        "Logic error: Acquired pads count ({}) less than chunk index ({})",
                        acquired_pads.len(),
                        i
                    );
                    return Err(DataError::InternalError(
                        "Pad acquisition count mismatch".to_string(),
                    ));
                }
                let (pad_address, secret_key) = acquired_pads[i].clone(); // Clone Arc needed parts

                let pad_info = PadInfo {
                    address: pad_address,
                    chunk_index: i,
                    status: PadStatus::Generated, // Start as Generated
                };

                initial_pads.push(pad_info.clone());
                initial_pad_keys.insert(pad_address, secret_key.to_bytes().to_vec());

                debug!("Task: Write chunk {} to new pad {}", i, pad_address);
                tasks_to_run.push(PadTask::Write {
                    pad_info,                  // Move ownership
                    secret_key,                // Move ownership
                    chunk_data: chunk.clone(), // Clone chunk data
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
            current_key_info = key_info; // Use the newly created info
        }
    }

    // --- Callback for Starting ---
    // Sent after initial setup (new or resume) is done and tasks are prepared
    let total_tasks = tasks_to_run.len(); // Number of actual writes/confirms needed
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

    // --- Concurrent Processing (Steps 3b onwards) ---
    debug!("Starting concurrent processing of {} tasks...", total_tasks);
    let mut write_futures = FuturesUnordered::new();
    let mut operation_error: Option<DataError> = None; // Store first error encountered
    let mut callback_cancelled = false;

    // Populate FuturesUnordered from the prepared tasks
    for task in tasks_to_run {
        match task {
            PadTask::Write {
                pad_info,
                secret_key,
                chunk_data,
            } => {
                let storage_manager = Arc::clone(&deps.storage_manager);
                let user_key_clone = user_key.clone();
                let index_manager = Arc::clone(&deps.index_manager);
                // TODO: Add existence check here if required by plan?
                // let network_adapter = Arc::clone(&deps.network_adapter);

                write_futures.push(async move {
                    let write_result = storage_manager
                        .write_pad_data(&secret_key, &chunk_data)
                        .await;

                    // Return pad info along with result for context
                    (pad_info, write_result)
                });
            }
        }
    }

    // Process completed futures
    let mut processed_count = 0;
    while let Some((pad_info, result)) = write_futures.next().await {
        processed_count += 1;
        match result {
            Ok(written_address) => {
                // Sanity check: ensure returned address matches expected
                if written_address != pad_info.address {
                    warn!(
                         "StorageManager returned address {} but expected {}. Using returned address.",
                         written_address, pad_info.address
                     );
                    // Update pad_info with the actual address if needed, though KeyInfo holds the source of truth
                }

                trace!(
                    "Successfully wrote chunk {} to pad {}",
                    pad_info.chunk_index,
                    written_address // Use the returned address
                );

                // Update pad status in index
                match deps
                    .index_manager
                    .update_pad_status(&user_key, &written_address, PadStatus::Written)
                    .await
                {
                    Ok(_) => {
                        trace!("Updated status to Written for pad {}", written_address);
                        // Update local copy of KeyInfo status for final check?
                        // It's simpler to re-fetch KeyInfo at the end if needed.
                    }
                    Err(e) => {
                        error!(
                            "Failed to update pad status to Written for {}: {}. Halting operation.",
                            written_address, e
                        );
                        operation_error = Some(DataError::Index(e));
                        break; // Stop processing further tasks
                    }
                }

                // Invoke callback
                if !invoke_put_callback(
                    &mut callback,
                    PutEvent::ChunkWritten {
                        chunk_index: pad_info.chunk_index,
                    },
                )
                .await
                .map_err(|e| {
                    DataError::InternalError(format!("Callback invocation failed: {}", e))
                })? {
                    error!("Store operation cancelled by callback during chunk writing.");
                    operation_error = Some(DataError::OperationCancelled);
                    callback_cancelled = true;
                    // No need to release pads explicitly here.
                    break; // Stop processing
                }
            }
            Err(e) => {
                error!(
                    "Failed to write chunk {} to pad {}: {}. Halting operation.",
                    pad_info.chunk_index, pad_info.address, e
                );
                // Store the first error and stop processing further tasks
                if operation_error.is_none() {
                    operation_error = Some(DataError::Storage(e.into()));
                }
                break; // Stop processing
            }
        }
        // Break loop immediately if an error or cancellation occurred
        if operation_error.is_some() {
            break;
        }
    }

    // Abort remaining futures if cancelled or error occurred
    if callback_cancelled || operation_error.is_some() {
        debug!("Aborting remaining write tasks due to error or cancellation.");
        // Drop remaining futures instead of abort_all()
        while write_futures.next().await.is_some() {
            // Consuming the futures drops them
        }
    }

    // --- Final Check and Completion ---

    // If an error occurred or was cancelled, return the error
    if let Some(err) = operation_error {
        warn!("Store operation for key '{}' failed: {}", user_key, err);
        // The index state reflects the progress made before the error.
        return Err(err);
    }

    // If we finished processing all tasks without error, check if the key is fully complete
    // Re-fetch the latest KeyInfo to be absolutely sure of the state
    let final_key_info = deps
        .index_manager
        .get_key_info(&user_key)
        .await?
        .ok_or_else(|| {
            DataError::InternalError(format!(
                "KeyInfo for '{}' disappeared unexpectedly after writes",
                user_key
            ))
        })?;

    let all_pads_written = final_key_info
        .pads
        .iter()
        .all(|p| p.status == PadStatus::Written || p.status == PadStatus::Confirmed);

    if all_pads_written && !final_key_info.is_complete {
        debug!(
            "All pads for key '{}' are now Written. Marking key as complete.",
            user_key
        );
        deps.index_manager.mark_key_complete(&user_key).await?;
    } else if !all_pads_written {
        warn!("Store operation for key '{}' finished processing tasks, but not all pads reached Written state. Key remains incomplete.", user_key);
        // This case might happen if some tasks were confirmations and failed, or if there's a logic bug.
    }

    // Send Complete callback only if the operation finished without external error/cancellation
    info!(
        "Store operation processing finished for key '{}'.",
        user_key
    );
    if !invoke_put_callback(&mut callback, PutEvent::Complete)
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
