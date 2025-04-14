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
use tokio::sync::Mutex; // Correct import for async Mutex
use tokio::sync::Semaphore; // Added
use tokio::task::JoinSet; // Added

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
    mut callback: Option<PutCallback>,
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
                return execute_upload_phase(
                    ma,
                    &key_owned,
                    data_bytes,
                    scratchpad_size,
                    callback,
                    pending_pads_count,
                )
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
        // Emit ReservingPads event before generating
        invoke_callback(
            &mut callback, // Borrow mutably here
            PutEvent::ReservingPads {
                count: new_pads_needed as u64,
            },
        )
        .await?;

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
    execute_upload_phase(
        ma,
        &key_owned,
        data_bytes,
        scratchpad_size,
        callback,
        new_pads_needed,
    )
    .await
}

const MAX_CONCURRENT_WRITES: usize = 10;

/// Handles the iterative process of uploading data chunks based on PadInfo status.
async fn execute_upload_phase(
    ma: &MutAnt,
    key: &str, // Use borrowed str
    data_bytes: &[u8],
    scratchpad_size: usize,
    callback: Option<PutCallback>,
    _total_new_pads_to_reserve: usize, // No longer needed for ReservationProgress
) -> Result<(), Error> {
    let key_owned = key.to_string();
    let data_chunks = Arc::new(chunk_data(data_bytes, scratchpad_size));
    let total_pads_expected = data_chunks.len();

    // --- Create Shared state for callbacks ---
    let callback_arc = Arc::new(Mutex::new(callback));
    let total_pads_committed_arc = Arc::new(Mutex::new(0u64));
    let bytes_written_arc = Arc::new(Mutex::new(0u64)); // Track total bytes written
                                                        // --- End Shared state ---

    // Emit StartingUpload event
    let total_bytes_expected = data_bytes.len() as u64;
    invoke_callback(
        &mut *callback_arc.lock().await,
        PutEvent::StartingUpload {
            total_bytes: total_bytes_expected,
        },
    )
    .await?;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_WRITES));
    let mut join_set: JoinSet<Result<(usize, u64), Error>> = JoinSet::new(); // Returns (index, chunk_size) on success
    let mut pads_to_process_count = 0;
    let mut first_error: Option<Error> = None;

    // --- Single Concurrent Processing Phase ---
    debug!(
        "ExecuteUpload[{}]: Processing non-Populated pads...",
        key_owned
    );
    {
        let mis_guard = ma.master_index_storage.lock().await;
        if let Some(key_info) = mis_guard.index.get(&key_owned) {
            for (pad_index, pad_info) in key_info.pads.iter().enumerate() {
                if pad_info.status == PadUploadStatus::Populated {
                    continue; // Skip already done pads
                }

                if pad_index >= data_chunks.len() {
                    error!(
                        "ExecuteUpload[{}]: Pad index {} out of bounds for data chunks (len {}). Skipping.",
                        key_owned, pad_index, data_chunks.len()
                    );
                    continue;
                }

                pads_to_process_count += 1;
                let is_new_pad = pad_info.status == PadUploadStatus::Generated;
                let chunk = data_chunks[pad_index].clone();
                let chunk_size = chunk.len() as u64;
                let pad_info_clone = pad_info.clone(); // Clone needed PadInfo for task
                let ma_storage = ma.storage.clone();
                let key_for_task = key_owned.clone();
                let cb_arc_clone = callback_arc.clone();
                let commit_arc_clone = total_pads_committed_arc.clone();
                let sem_clone = semaphore.clone();
                let bytes_written_clone = bytes_written_arc.clone(); // Clone for progress reporting

                join_set.spawn(async move {
                    let permit = sem_clone.acquire_owned().await.expect("Semaphore closed");
                    debug!(
                        "ExecuteUploadTask[{}]: Acquired permit. Writing pad index {} (IsNew: {})",
                        key_for_task, pad_index, is_new_pad
                    );

                    let write_result = pad_manager::write::write_chunk(
                        &ma_storage,
                        pad_info_clone.address,
                        &pad_info_clone.key,
                        &chunk,
                        is_new_pad,
                        &key_for_task,
                        pad_index,
                        &cb_arc_clone,
                        &commit_arc_clone,
                        total_pads_expected,
                        scratchpad_size,
                    )
                    .await;

                    drop(permit);

                    match write_result {
                        Ok(_) => {
                            // Update total bytes written
                            let current_bytes = {
                                let mut guard = bytes_written_clone.lock().await;
                                *guard += chunk_size;
                                *guard
                            };
                            // Emit UploadProgress incrementally
                            invoke_callback(
                                &mut *cb_arc_clone.lock().await,
                                PutEvent::UploadProgress {
                                    bytes_written: current_bytes,
                                    total_bytes: total_bytes_expected,
                                },
                            )
                            .await
                            .ok(); // Ignore callback errors within task?

                            Ok((pad_index, chunk_size))
                        }
                        Err(e) => {
                            error!(
                                "ExecuteUploadTask[{}]: write_chunk failed for index {}: {}",
                                key_for_task, pad_index, e
                            );
                            Err(e)
                        }
                    }
                });
            }
        } else {
            warn!(
                "ExecuteUpload[{}]: Key not found during task spawning phase.",
                key_owned
            );
            return Err(Error::KeyNotFound(key_owned));
        }
    } // Release MIS lock after spawning tasks

    // Collect results and update state incrementally
    let mut successful_updates = 0;
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok((pad_index, _chunk_size))) => {
                successful_updates += 1;
                // Update status for this specific pad
                let mut mis_guard = ma.master_index_storage.lock().await;
                if let Some(key_info) = mis_guard.index.get_mut(&key_owned) {
                    if let Some(pi_mut) = key_info.pads.get_mut(pad_index) {
                        if pi_mut.status != PadUploadStatus::Populated {
                            pi_mut.status = PadUploadStatus::Populated;
                            key_info.modified = Utc::now();
                            // Persist after each successful update
                            let network_choice = ma.storage.get_network_choice();
                            let mis_data_to_write = mis_guard.clone();
                            drop(mis_guard); // Drop lock before write
                            if let Err(e) =
                                write_local_index(&mis_data_to_write, network_choice).await
                            {
                                error!(
                                    "ExecuteUpload[{}][Pad {}]: Failed to persist state after update: {}. Continuing...",
                                    key_owned, pad_index, e
                                );
                            }
                        } else {
                            drop(mis_guard);
                        } // Drop lock if no update needed
                    } else {
                        drop(mis_guard);
                    } // Drop lock if pad index invalid
                } else {
                    drop(mis_guard);
                } // Drop lock if key invalid
            }
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
                join_set.abort_all(); // Abort others on first error
            }
            Err(je) => {
                error!("ExecuteUpload[{}]: JoinError: {}", key_owned, je);
                if first_error.is_none() {
                    first_error = Some(Error::JoinError(je.to_string()));
                }
                join_set.abort_all();
            }
        }
    }

    if let Some(e) = first_error.take() {
        return Err(e);
    }

    // Final check
    if successful_updates == pads_to_process_count {
        info!(
            "ExecuteUpload[{}]: All {} pads processed successfully. Upload complete.",
            key_owned, successful_updates
        );
        invoke_callback(&mut *callback_arc.lock().await, PutEvent::UploadFinished).await?;
        invoke_callback(&mut *callback_arc.lock().await, PutEvent::StoreComplete).await?;
        Ok(())
    } else {
        error!(
            "ExecuteUpload[{}]: Upload finished, but only {}/{} pads succeeded. State inconsistent.",
            key_owned, successful_updates, pads_to_process_count
        );
        Err(Error::InternalError(format!(
            "Upload finished with incomplete pads for key {}",
            key_owned
        )))
    }
}
