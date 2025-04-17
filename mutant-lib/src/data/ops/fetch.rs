// Fetch operation logic
use crate::data::chunking::reassemble_data;
use crate::data::error::DataError;
use crate::events::{invoke_get_callback, GetCallback, GetEvent};
// Remove unused import
// use crate::storage::StorageManager;
use autonomi::SecretKey;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
// Needed for pad_keys lookup
use std::sync::Arc;
// Only needed if using tokio specifics like time::sleep

use super::common::DataManagerDependencies;

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
