use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::events::{
    invoke_get_callback, invoke_put_callback, GetCallback, GetEvent, PutCallback, PutEvent,
};
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
    // Add master index address/key if needed for saving index directly?
    // No, IndexManager::save should encapsulate that.
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

    // 1. Get chunk size and chunk data
    let chunk_size = deps.index_manager.get_scratchpad_size().await?;
    let chunks = chunk_data(data_bytes, chunk_size)?;
    let num_chunks = chunks.len();
    debug!("Data chunked into {} pieces.", num_chunks);

    if !invoke_put_callback(
        &mut callback,
        PutEvent::Starting {
            total_chunks: num_chunks,
        },
    )
    .await
    .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        return Err(DataError::OperationCancelled);
    }

    // Check if key already exists *before* acquiring pads
    if deps.index_manager.get_key_info(&user_key).await?.is_some() {
        info!(
            "Key '{}' already exists. Use --force to overwrite.",
            user_key
        );
        return Err(DataError::KeyAlreadyExists(user_key));
    }

    // Handle empty data case: store metadata but no pads
    if num_chunks == 0 {
        debug!("Storing empty data for key '{}'", user_key);
        let key_info = KeyInfo {
            pads: Vec::new(),
            pad_keys: HashMap::new(), // Empty map for empty data
            data_size,
            modified: Utc::now(),
            is_complete: true,
            populated_pads_count: 0,
        };
        deps.index_manager
            .insert_key_info(user_key, key_info)
            .await?;
        // Save index immediately for empty data? Yes, seems consistent.
        // Need master index address/key here... This suggests IndexManager::save needs adjustment
        // or these details need to be passed down. Let's assume IndexManager handles it for now.
        // deps.index_manager.save(???, ???).await?; // How to get address/key?
        // TODO: Revisit index saving strategy. For now, skip explicit save here. API layer will save.
        if !invoke_put_callback(&mut callback, PutEvent::Complete)
            .await
            .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
        {
            return Err(DataError::OperationCancelled);
        }
        return Ok(());
    }

    // 2. Acquire necessary pads
    debug!("Acquiring {} pads...", num_chunks);
    let acquired_pads = deps.pad_lifecycle_manager.acquire_pads(num_chunks).await?;
    if acquired_pads.len() < num_chunks {
        // Should not happen if acquire_pads works correctly, but check defensively
        error!(
            "Acquired {} pads, but {} were needed. Releasing acquired pads.",
            acquired_pads.len(),
            num_chunks
        );
        // Release the partially acquired pads - requires keys map
        let keys_map: HashMap<_, _> = acquired_pads
            .iter()
            .map(|(a, k)| (*a, k.to_bytes().to_vec()))
            .collect();
        let pad_infos_to_release = acquired_pads
            .iter()
            .map(|(a, _)| PadInfo {
                address: *a,
                chunk_index: 0,
            })
            .collect(); // chunk_index doesn't matter here
        if let Err(e) = deps
            .pad_lifecycle_manager
            .release_pads(pad_infos_to_release, &keys_map)
            .await
        {
            warn!(
                "Failed to release partially acquired pads during store failure: {}",
                e
            );
        }
        return Err(DataError::InsufficientFreePads(format!(
            "Needed {} pads, but only {} were available/acquired",
            num_chunks,
            acquired_pads.len()
        )));
    }
    debug!("Successfully acquired {} pads.", acquired_pads.len());

    // 3. Write chunks concurrently
    let mut write_futures = FuturesUnordered::new();
    let mut pad_info_list = Vec::with_capacity(num_chunks);
    let mut populated_count = 0;

    for (i, chunk) in chunks.into_iter().enumerate() {
        let (pad_address, pad_key) = acquired_pads[i].clone(); // Clone Arc'd key/address
        let storage_manager = Arc::clone(&deps.storage_manager);
        pad_info_list.push(PadInfo {
            address: pad_address,
            chunk_index: i,
        });

        write_futures.push(async move {
            let result = storage_manager.write_pad_data(&pad_key, &chunk).await;
            (i, result) // Return index and Result<ScratchpadAddress, StorageError>
        });
    }

    // PadInfo list needs to be built after writes complete
    let mut final_pad_info_list = vec![None; num_chunks];

    while let Some((chunk_index, result)) = write_futures.next().await {
        match result {
            Ok(written_address) => {
                populated_count += 1;
                trace!(
                    "Successfully wrote chunk {} to pad {}",
                    chunk_index,
                    written_address // Use the returned address
                );
                // Store the PadInfo for the index later
                if chunk_index < final_pad_info_list.len() {
                    final_pad_info_list[chunk_index] = Some(PadInfo {
                        address: written_address,
                        chunk_index,
                    });
                } else {
                    error!(
                        "Invalid chunk index {} returned during store write (max expected {})",
                        chunk_index,
                        num_chunks - 1
                    );
                    // TODO: Handle cancellation/failure more robustly
                    return Err(DataError::InternalError(format!(
                        "Invalid chunk index {} encountered during store",
                        chunk_index
                    )));
                }

                if !invoke_put_callback(&mut callback, PutEvent::ChunkWritten { chunk_index })
                    .await
                    .map_err(|e| {
                        DataError::InternalError(format!("Callback invocation failed: {}", e))
                    })?
                {
                    error!("Store operation cancelled by callback during chunk writing.");
                    // Release ALL acquired pads if cancelled
                    let keys_map: HashMap<_, _> = acquired_pads
                        .iter()
                        .map(|(a, k)| (*a, k.to_bytes().to_vec()))
                        .collect();
                    if let Err(e) = deps
                        .pad_lifecycle_manager
                        .release_pads(pad_info_list, &keys_map)
                        .await
                    {
                        warn!("Failed to release pads after store cancellation: {}", e);
                    }
                    return Err(DataError::OperationCancelled);
                }
            }
            Err(e) => {
                // Get the address associated with this failed chunk index for logging
                let failed_address_str = final_pad_info_list
                    .get(chunk_index)
                    .and_then(|opt| opt.as_ref())
                    .map(|pi| pi.address.to_string())
                    .unwrap_or_else(|| "<unknown address>".to_string()); // Log string instead
                error!(
                    "Failed to write chunk {} to pad {}: {}",
                    chunk_index, failed_address_str, e
                );
                // TODO: Handle partial write failure (release pads, mark as incomplete?)
                // Release ALL acquired pads on failure
                let keys_map: HashMap<_, _> = acquired_pads
                    .iter()
                    .map(|(a, k)| (*a, k.to_bytes().to_vec()))
                    .collect();
                if let Err(rel_e) = deps
                    .pad_lifecycle_manager
                    .release_pads(pad_info_list, &keys_map)
                    .await
                {
                    warn!(
                        "Failed to release pads after store write failure: {}",
                        rel_e
                    );
                }
                return Err(DataError::Storage(e.into())); // Convert StorageError
            }
        }
    }

    debug!("All {} chunks written successfully.", num_chunks);

    // Collect the final PadInfo list, ensuring all slots are filled
    let pad_info_list: Vec<PadInfo> = final_pad_info_list
        .into_iter()
        .map(|opt| opt.expect("Missing PadInfo after successful write"))
        .collect();

    // Prepare pad keys map for KeyInfo
    let pad_keys_map: HashMap<ScratchpadAddress, Vec<u8>> = acquired_pads
        .into_iter()
        .map(|(addr, key)| (addr, key.to_bytes().to_vec()))
        .collect();

    // 4. Update index
    let key_info = KeyInfo {
        pads: pad_info_list, // Already ordered by chunk index
        pad_keys: pad_keys_map,
        data_size,
        modified: Utc::now(),
        is_complete: true, // Assuming all writes succeeded if we reached here
        populated_pads_count: populated_count,
    };

    deps.index_manager
        .insert_key_info(user_key.clone(), key_info)
        .await?;
    debug!("Index updated for key '{}'", user_key);

    // 5. Save index (explicitly triggered by API layer, not here)
    // if !invoke_put_callback(&mut callback, PutEvent::SavingIndex).await? {
    //     return Err(DataError::OperationCancelled);
    // }
    // deps.index_manager.save(???, ???).await?; // How to get address/key?

    if !invoke_put_callback(&mut callback, PutEvent::Complete)
        .await
        .map_err(|e| DataError::InternalError(format!("Callback invocation failed: {}", e)))?
    {
        return Err(DataError::OperationCancelled);
    }

    info!("DataOps: Store operation complete for key '{}'", user_key);
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
        Some(key_info) => {
            debug!("Removed key info for '{}' from index.", user_key);
            // 2. Release associated pads
            if !key_info.pads.is_empty() {
                debug!("Releasing {} associated pads...", key_info.pads.len());
                // Use the keys stored within the KeyInfo itself
                if !key_info.pad_keys.is_empty() {
                    deps.pad_lifecycle_manager
                        .release_pads(key_info.pads, &key_info.pad_keys) // Pass pad_keys directly
                        .await?; // Propagate error if release fails
                } else {
                    warn!(
                        "Pad keys missing in KeyInfo for key '{}'. Cannot release pads.",
                        user_key
                    );
                    // Should this be an error? Maybe InternalError?
                }
            } else {
                debug!("No pads associated with key '{}' to release.", user_key);
            }

            // 3. Save index (explicitly triggered by API layer)
            // deps.index_manager.save(???, ???).await?;

            info!("DataOps: Remove operation complete for key '{}'", user_key);
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

// --- Update Operation ---
// TODO: Implement update_op. This is complex:
// 1. Fetch existing KeyInfo. Error if not found.
// 2. Chunk new data.
// 3. Compare new chunk count with old pad count.
// 4. If counts differ:
//    - Acquire new pads if needed.
//    - Identify pads to release if shrinking. Need keys for release!
// 5. Write new/updated chunks concurrently (potentially overwriting existing pads).
// 6. Release any now-unused pads.
// 7. Update KeyInfo (new size, timestamp, potentially new pad list).
// 8. Update index.
// 9. Save index (via API layer).
// Requires careful handling of partial failures and pad key management.

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

// --- Helper: Checksum Calculation --- (assuming this is still needed elsewhere or can be removed later)
// ... existing code ...
