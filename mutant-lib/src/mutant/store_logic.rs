use super::MutAnt;
use crate::cache::write_local_index; // Needed for persistence
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::{KeyStorageInfo, PadInfo, PadUploadStatus};
use crate::pad_manager;
use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use hex;
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Calculates the SHA256 checksum of the data and returns it as a hex string.
fn calculate_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Splits data into chunks based on scratchpad size.
fn chunk_data(data: &[u8], chunk_size: usize) -> Vec<Vec<u8>> {
    if chunk_size == 0 {
        // Avoid division by zero, though this should be caught earlier.
        error!("Chunk size is zero, cannot chunk data.");
        return Vec::new(); // Or handle as error
    }
    data.chunks(chunk_size)
        .map(|chunk| chunk.to_vec())
        .collect()
}

/// Stores data under a given key, implementing resumable uploads.
pub(super) async fn store_data(
    ma: &MutAnt,
    key: &str,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), Error> {
    let data_size = data_bytes.len();
    let key_owned = key.to_string();
    trace!(
        "MutAnt::StoreData[{}]: Starting for key '{}' ({} bytes)",
        ma.master_index_addr,
        key, // Use the borrowed key for logging
        data_size
    );

    let new_checksum = calculate_checksum(data_bytes);
    debug!(
        "StoreData[{}]: Calculated new data checksum: {}",
        key_owned, new_checksum
    );

    let mut mis_lock = ma.master_index_storage.lock().await;
    let scratchpad_size = mis_lock.scratchpad_size;
    if scratchpad_size == 0 {
        return Err(Error::InternalError(
            "MasterIndexStorage scratchpad_size is zero".to_string(),
        ));
    }

    let existing_key_info = mis_lock.index.get_mut(&key_owned);

    // --- Entry Point & Resume Logic ---
    if let Some(key_info) = existing_key_info {
        debug!(
            "StoreData[{}]: Key exists. Current checksum: {}. New checksum: {}.",
            key_owned, key_info.data_checksum, new_checksum
        );
        if key_info.data_checksum == new_checksum && key_info.data_size == data_size {
            // Check if already fully populated
            if key_info
                .pads
                .iter()
                .all(|p| p.status == PadUploadStatus::Populated)
            {
                info!(
                    "StoreData[{}]: Data checksum matches and all pads populated. Nothing to do.",
                    key_owned
                );
                return Ok(());
            }

            // Resume logic: Find pending pads and jump to Upload Execution
            let pending_pads_count = key_info
                .pads
                .iter()
                .filter(|p| {
                    p.status == PadUploadStatus::Generated || p.status == PadUploadStatus::Free
                })
                .count();

            if pending_pads_count > 0 {
                info!(
                    "StoreData[{}]: Checksum matches. Resuming upload for {} pending pads.",
                    key_owned, pending_pads_count
                );
                // Release lock before entering upload phase
                drop(mis_lock);
                // Jump to Upload Execution Phase (using the existing key info)
                return execute_upload_phase(ma, &key_owned, data_bytes, scratchpad_size, callback)
                    .await;
            } else {
                // Should technically be caught by the 'all populated' check above, but log defensively.
                warn!(
                    "StoreData[{}]: Checksum matches but no pending pads found, yet not all are Populated? State inconsistent.",
                    key_owned
                );
                // Proceed as if it's a new upload, potentially overwriting.
                // Or return an error? For now, proceed to planning.
            }
        } else {
            // Checksum/size differs - Treat as update (delete + store)
            // TODO: Implement proper update logic later. For now, error out or delete.
            warn!(
                "StoreData[{}]: Data checksum or size differs. Treating as new store (overwrite). Current logic will delete existing first.",
                key_owned
            );
            // Drop the mutable borrow before calling remove
            drop(mis_lock);
            // Call remove logic (needs MutAnt instance, key)
            super::remove_logic::delete_item(ma, &key_owned).await?; // Use delete_item with 2 args
                                                                     // Re-acquire lock for planning phase
            mis_lock = ma.master_index_storage.lock().await;
            // Fall through to Planning Phase
        }
    } else {
        debug!(
            "StoreData[{}]: Key does not exist. Proceeding to planning.",
            key_owned
        );
        // Fall through to Planning Phase
    }

    // --- Planning Phase --- (Executed for new keys or differing checksums)
    info!("StoreData[{}]: Entering Planning Phase.", key_owned);
    let data_chunks = chunk_data(data_bytes, scratchpad_size);
    let needed_total_pads = data_chunks.len();
    debug!(
        "StoreData[{}]: Need {} total pads for {} chunks.",
        key_owned,
        needed_total_pads,
        data_chunks.len()
    );

    let mut planned_pads: Vec<PadInfo> = Vec::with_capacity(needed_total_pads);

    // 1. Reuse free pads
    let reusable_pads_count = std::cmp::min(needed_total_pads, mis_lock.free_pads.len());
    if reusable_pads_count > 0 {
        debug!(
            "StoreData[{}]: Reusing {} pads from free list.",
            key_owned, reusable_pads_count
        );
        let start_index = mis_lock.free_pads.len() - reusable_pads_count;
        let pads_to_reuse: Vec<(ScratchpadAddress, Vec<u8>)> =
            mis_lock.free_pads.drain(start_index..).collect();

        for (address, key) in pads_to_reuse {
            planned_pads.push(PadInfo {
                address,
                key,
                status: PadUploadStatus::Free, // Reused pads are reserved, need data write
                is_new: false,                 // Not new, already exists
            });
        }
    }

    // 2. Generate new pads if needed
    let new_pads_needed = needed_total_pads - reusable_pads_count;
    if new_pads_needed > 0 {
        debug!(
            "StoreData[{}]: Generating {} new pads.",
            key_owned, new_pads_needed
        );
        for _ in 0..new_pads_needed {
            let new_key = SecretKey::random();
            let new_address = ScratchpadAddress::new(new_key.public_key().into());
            planned_pads.push(PadInfo {
                address: new_address,
                key: new_key.to_bytes().to_vec(), // Store key bytes
                status: PadUploadStatus::Generated, // Need initial create + write
                is_new: true,
            });
        }
    }

    // 3. Create/Update KeyStorageInfo
    let new_key_info = KeyStorageInfo {
        pads: planned_pads,
        data_size,
        data_checksum: new_checksum,
        modified: Utc::now(),
    };

    // 4. Persist MasterIndexStorage locally BEFORE starting uploads
    mis_lock.index.insert(key_owned.clone(), new_key_info);
    debug!(
        "StoreData[{}]: Updated MasterIndex locally with planned pads. Persisting...",
        key_owned
    );
    let network_choice = ma.storage.get_network_choice();
    // Clone the MIS data to write it *after* releasing the lock
    let mis_data_to_write = mis_lock.clone();
    drop(mis_lock); // Release lock

    match write_local_index(&mis_data_to_write, network_choice).await {
        Ok(_) => {
            info!(
                "StoreData[{}]: Successfully persisted planned state.",
                key_owned
            );
        }
        Err(e) => {
            error!(
                "StoreData[{}]: CRITICAL: Failed to persist planned state: {}. Aborting.",
                key_owned, e
            );
            // Depending on requirements, might try to rollback or just error
            return Err(Error::CacheError(format!(
                "Failed to persist planned state for key {}: {}",
                key_owned, e
            )));
        }
    }

    // --- Upload Execution Phase --- (Called after planning or directly on resume)
    info!("StoreData[{}]: Entering Upload Execution Phase.", key_owned);
    execute_upload_phase(ma, &key_owned, data_bytes, scratchpad_size, callback).await
}

/// Handles the iterative process of uploading data chunks based on PadInfo status.
async fn execute_upload_phase(
    ma: &MutAnt,
    key: &str, // Use borrowed str
    data_bytes: &[u8],
    scratchpad_size: usize,
    mut callback: Option<PutCallback>,
) -> Result<(), Error> {
    let key_owned = key.to_string(); // Clone for async boundaries if needed
    let data_chunks = Arc::new(chunk_data(data_bytes, scratchpad_size));

    // Emit StartingUpload event
    let total_bytes_expected = data_bytes.len() as u64;
    invoke_callback(
        &mut callback,
        PutEvent::StartingUpload {
            total_bytes: total_bytes_expected,
        },
    )
    .await?;

    let _total_pads = data_chunks.len(); // Prefix with underscore
    let mut pads_processed = 0;

    // --- Phase 1: Process Generated Pads (Create + Write) ---
    debug!(
        "ExecuteUpload[{}]: Processing 'Generated' pads...",
        key_owned
    );
    loop {
        let pad_to_process: Option<(usize, PadInfo)>;
        {
            let mis_guard = ma.master_index_storage.lock().await;
            // Find the *first* 'Generated' pad in the current list
            pad_to_process = mis_guard.index.get(&key_owned).and_then(|ki| {
                ki.pads.iter().enumerate().find_map(|(idx, pi)| {
                    if pi.status == PadUploadStatus::Generated {
                        Some((idx, pi.clone())) // Clone needed PadInfo
                    } else {
                        None
                    }
                })
            });
        } // Lock released

        if let Some((pad_index, pad_info)) = pad_to_process {
            if pad_index >= data_chunks.len() {
                error!(
                    "ExecuteUpload[{}]: Pad index {} out of bounds for data chunks (len {}). Aborting.",
                    key_owned, pad_index, data_chunks.len()
                );
                return Err(Error::InternalError(format!(
                    "Data chunk index out of bounds for key {}",
                    key_owned
                )));
            }
            let chunk = &data_chunks[pad_index];
            debug!(
                "ExecuteUpload[{}]: Attempting write_chunk (create) for pad index {} (Address: {}).",
                key_owned, pad_index, pad_info.address
            );

            // TODO: Add callback for UploadProgress / ScratchpadCommitStart?

            match pad_manager::write::write_chunk(
                &ma.storage,
                pad_info.address,
                &pad_info.key,
                chunk,
                pad_info.is_new, // Should be true for Generated
            )
            .await
            {
                Ok(_) => {
                    info!(
                        "ExecuteUpload[{}]: write_chunk (create) succeeded for pad index {}. Updating status to Free.",
                        key_owned, pad_index
                    );
                    // Update status in MIS and persist
                    let mut mis_guard = ma.master_index_storage.lock().await;
                    if let Some(key_info) = mis_guard.index.get_mut(&key_owned) {
                        if let Some(pi_mut) = key_info.pads.get_mut(pad_index) {
                            pi_mut.status = PadUploadStatus::Free;
                            key_info.modified = Utc::now();
                        } else {
                            error!(
                                "ExecuteUpload[{}]: Pad index {} not found during status update (Generated -> Free). Inconsistency?",
                                key_owned, pad_index
                            );
                            // Continue for now, but log error
                        }
                        // Persist after update
                        let network_choice = ma.storage.get_network_choice();
                        let mis_data_to_write = mis_guard.clone();
                        drop(mis_guard);
                        if let Err(e) = write_local_index(&mis_data_to_write, network_choice).await
                        {
                            error!(
                                "ExecuteUpload[{}]: Failed to persist state after pad {} status update (Generated -> Free): {}. Continuing...",
                                key_owned, pad_index, e
                            );
                            // Do not return error here, allow upload to proceed if possible
                        }
                    } else {
                        error!(
                            "ExecuteUpload[{}]: Key not found during status update (Generated -> Free). Inconsistency?",
                            key_owned
                        );
                        // Drop lock if held
                    }
                    // TODO: Add callback for ScratchpadCommitComplete?
                }
                Err(e) => {
                    error!(
                        "ExecuteUpload[{}]: write_chunk (create) failed for pad index {}: {}. Aborting.",
                        key_owned, pad_index, e
                    );
                    // TODO: Implement retry logic or better error handling?
                    return Err(e); // Abort on first failure for now
                }
            }
        } else {
            debug!(
                "ExecuteUpload[{}]: No more 'Generated' pads found.",
                key_owned
            );
            break; // Exit loop when no more Generated pads are found
        }
    }

    // --- Phase 2: Process Free Pads (Update + Write) ---
    debug!("ExecuteUpload[{}]: Processing 'Free' pads...", key_owned);
    loop {
        let pad_to_process: Option<(usize, PadInfo)>;
        {
            let mis_guard = ma.master_index_storage.lock().await;
            pad_to_process = mis_guard.index.get(&key_owned).and_then(|ki| {
                ki.pads.iter().enumerate().find_map(|(idx, pi)| {
                    if pi.status == PadUploadStatus::Free {
                        Some((idx, pi.clone()))
                    } else {
                        None
                    }
                })
            });
        } // Lock released

        if let Some((pad_index, pad_info)) = pad_to_process {
            if pad_index >= data_chunks.len() {
                error!(
                    "ExecuteUpload[{}]: Pad index {} out of bounds for data chunks (len {}). Aborting.",
                    key_owned, pad_index, data_chunks.len()
                );
                return Err(Error::InternalError(format!(
                    "Data chunk index out of bounds for key {}",
                    key_owned
                )));
            }
            let chunk = &data_chunks[pad_index];
            debug!(
                "ExecuteUpload[{}]: Attempting write_chunk (update) for pad index {} (Address: {}).",
                key_owned, pad_index, pad_info.address
            );

            // TODO: Add callback for UploadProgress?

            match pad_manager::write::write_chunk(
                &ma.storage,
                pad_info.address,
                &pad_info.key,
                chunk,
                false, // Explicitly pass false for is_new when status is Free
            )
            .await
            {
                Ok(_) => {
                    info!(
                        "ExecuteUpload[{}]: write_chunk (update) succeeded for pad index {}. Updating status to Populated.",
                        key_owned, pad_index
                    );
                    pads_processed += 1;
                    // Calculate bytes written based on pads processed and scratchpad size
                    // This is an approximation, assumes full pads except maybe the last.
                    let bytes_written =
                        std::cmp::min(pads_processed * scratchpad_size, data_bytes.len()) as u64;
                    let total_bytes = data_bytes.len() as u64;

                    // Use correct fields for UploadProgress
                    invoke_callback(
                        &mut callback,
                        PutEvent::UploadProgress {
                            bytes_written,
                            total_bytes,
                        },
                    )
                    .await?;

                    // Update status in MIS and persist
                    let mut mis_guard = ma.master_index_storage.lock().await;
                    if let Some(key_info) = mis_guard.index.get_mut(&key_owned) {
                        if let Some(pi_mut) = key_info.pads.get_mut(pad_index) {
                            pi_mut.status = PadUploadStatus::Populated;
                            key_info.modified = Utc::now();
                        } else {
                            error!(
                                "ExecuteUpload[{}]: Pad index {} not found during status update (Free -> Populated). Inconsistency?",
                                key_owned, pad_index
                            );
                        }
                        // Persist after update
                        let network_choice = ma.storage.get_network_choice();
                        let mis_data_to_write = mis_guard.clone();
                        drop(mis_guard);
                        if let Err(e) = write_local_index(&mis_data_to_write, network_choice).await
                        {
                            error!(
                                "ExecuteUpload[{}]: Failed to persist state after pad {} status update (Free -> Populated): {}. Continuing...",
                                key_owned, pad_index, e
                            );
                        }
                    } else {
                        error!(
                            "ExecuteUpload[{}]: Key not found during status update (Free -> Populated). Inconsistency?",
                            key_owned
                        );
                    }
                }
                Err(e) => {
                    error!(
                        "ExecuteUpload[{}]: write_chunk (update) failed for pad index {}: {}. Aborting.",
                        key_owned, pad_index, e
                    );
                    return Err(e); // Abort on first failure
                }
            }
        } else {
            debug!("ExecuteUpload[{}]: No more 'Free' pads found.", key_owned);
            break; // Exit loop
        }
    }

    // Final check: Ensure all pads are populated
    let mis_guard = ma.master_index_storage.lock().await;
    if let Some(key_info) = mis_guard.index.get(&key_owned) {
        if key_info
            .pads
            .iter()
            .all(|p| p.status == PadUploadStatus::Populated)
        {
            info!(
                "ExecuteUpload[{}]: All pads processed and confirmed Populated. Upload complete.",
                key_owned
            );
            invoke_callback(&mut callback, PutEvent::UploadFinished).await?;
            Ok(())
        } else {
            let pending_count = key_info
                .pads
                .iter()
                .filter(|p| p.status != PadUploadStatus::Populated)
                .count();
            error!(
                "ExecuteUpload[{}]: Upload phase finished, but {} pads are still not Populated. State inconsistent.",
                key_owned, pending_count
            );
            Err(Error::InternalError(format!(
                "Upload finished with non-populated pads for key {}",
                key_owned
            )))
        }
    } else {
        error!(
            "ExecuteUpload[{}]: Key not found after upload phase. State inconsistent.",
            key_owned
        );
        Err(Error::InternalError(format!(
            "Key {} disappeared during upload",
            key_owned
        )))
    }
}
