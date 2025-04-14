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
    total_new_pads_to_reserve: usize,
) -> Result<(), Error> {
    let key_owned = key.to_string();
    let data_chunks = Arc::new(chunk_data(data_bytes, scratchpad_size));
    let total_pads_expected = data_chunks.len();

    // --- Create Shared state for callbacks ---
    let callback_arc = Arc::new(Mutex::new(callback));
    let total_pads_committed_arc = Arc::new(Mutex::new(0u64));
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
    let mut join_set: JoinSet<Result<usize, Error>> = JoinSet::new();
    let mut generated_pads_processed_count = 0; // Counter for reservation progress
    let mut bytes_written_during_create: u64 = 0; // Counter for upload progress during create phase
    let mut first_error: Option<Error> = None;

    // --- Phase 1: Process Generated Pads (Create + Write Concurrently) ---
    debug!(
        "ExecuteUpload[{}]: Processing 'Generated' pads...",
        key_owned
    );
    let generated_pads_to_process: Vec<(usize, PadInfo)> = {
        let mis_guard = ma.master_index_storage.lock().await;
        mis_guard
            .index
            .get(&key_owned)
            .map(|ki| {
                ki.pads
                    .iter()
                    .enumerate()
                    .filter(|(_, pi)| pi.status == PadUploadStatus::Generated)
                    .map(|(idx, pi)| (idx, pi.clone())) // Clone needed info
                    .collect()
            })
            .unwrap_or_default()
    };

    for (pad_index, pad_info) in generated_pads_to_process {
        if pad_index >= data_chunks.len() {
            error!(
                "ExecuteUpload[{}]: Pad index {} out of bounds for data chunks (len {}). Skipping.",
                key_owned,
                pad_index,
                data_chunks.len()
            );
            continue; // Should not happen if planning was correct
        }
        let chunk = data_chunks[pad_index].clone(); // Clone chunk for task
        let ma_storage = ma.storage.clone();
        let key_for_task = key_owned.clone();
        let cb_arc_clone = callback_arc.clone();
        let commit_arc_clone = total_pads_committed_arc.clone();
        let sem_clone = semaphore.clone();

        join_set.spawn(async move {
            let permit = sem_clone.acquire_owned().await.expect("Semaphore closed");
            debug!(
                "ExecuteUploadTask[{}][Generated]: Acquired permit. Writing pad index {}",
                key_for_task, pad_index
            );

            let write_result = pad_manager::write::write_chunk(
                &ma_storage,
                pad_info.address,
                &pad_info.key,
                &chunk,
                true, // is_new is true for Generated
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
                Ok(_) => Ok(pad_index), // Return index on success
                Err(e) => {
                    error!(
                        "ExecuteUploadTask[{}][Generated]: write_chunk failed for index {}: {}",
                        key_for_task, pad_index, e
                    );
                    Err(e)
                }
            }
        });
    }

    // Collect results for Generated phase
    let mut successfully_created_indices = Vec::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok(pad_index)) => {
                successfully_created_indices.push(pad_index);
                // Update counters for progress reporting incrementally
                generated_pads_processed_count += 1;
                let chunk_size_for_progress = if pad_index < data_chunks.len() {
                    data_chunks[pad_index].len() as u64
                } else {
                    0
                };
                bytes_written_during_create += chunk_size_for_progress;

                // Emit incremental progress
                if total_new_pads_to_reserve > 0 {
                    invoke_callback(
                        &mut *callback_arc.lock().await,
                        PutEvent::ReservationProgress {
                            current: generated_pads_processed_count as u64,
                            total: total_new_pads_to_reserve as u64,
                        },
                    )
                    .await?;
                }
                invoke_callback(
                    &mut *callback_arc.lock().await,
                    PutEvent::UploadProgress {
                        bytes_written: bytes_written_during_create,
                        total_bytes: total_bytes_expected,
                    },
                )
                .await?;
            }
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
                join_set.abort_all(); // Abort others on first error
            }
            Err(je) => {
                error!("ExecuteUpload[{}][Generated]: JoinError: {}", key_owned, je);
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

    // Batch update status and persist after successful Generated phase
    if !successfully_created_indices.is_empty() {
        debug!(
            "ExecuteUpload[{}][Generated]: Batch updating status for {} pads...",
            key_owned,
            successfully_created_indices.len()
        );
        let mut mis_guard = ma.master_index_storage.lock().await;
        let mut modified = false;
        if let Some(key_info) = mis_guard.index.get_mut(&key_owned) {
            for idx in successfully_created_indices {
                if let Some(pi_mut) = key_info.pads.get_mut(idx) {
                    if pi_mut.status == PadUploadStatus::Generated {
                        // Avoid double update
                        pi_mut.status = PadUploadStatus::Free;
                        modified = true;
                    }
                }
            }
            if modified {
                key_info.modified = Utc::now();
            }
        }
        if modified {
            let network_choice = ma.storage.get_network_choice();
            let mis_data_to_write = mis_guard.clone();
            drop(mis_guard); // Drop lock before write
            if let Err(e) = write_local_index(&mis_data_to_write, network_choice).await {
                error!(
                    "ExecuteUpload[{}][Generated]: Failed to persist state after batch update: {}. Continuing...",
                    key_owned, e
                );
            }
        }
    }

    // --- Phase 2: Process Free Pads (Update + Write Concurrently) ---
    debug!("ExecuteUpload[{}]: Processing 'Free' pads...", key_owned);
    let free_pads_to_process: Vec<(usize, PadInfo)> = {
        let mis_guard = ma.master_index_storage.lock().await;
        mis_guard
            .index
            .get(&key_owned)
            .map(|ki| {
                ki.pads
                    .iter()
                    .enumerate()
                    .filter(|(_, pi)| pi.status == PadUploadStatus::Free)
                    .map(|(idx, pi)| (idx, pi.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };

    for (pad_index, pad_info) in free_pads_to_process {
        if pad_index >= data_chunks.len() {
            error!(
                "ExecuteUpload[{}][Free]: Pad index {} out of bounds for data chunks (len {}). Skipping.",
                key_owned, pad_index, data_chunks.len()
            );
            continue;
        }
        let chunk = data_chunks[pad_index].clone();
        let ma_storage = ma.storage.clone();
        let key_for_task = key_owned.clone();
        let cb_arc_clone = callback_arc.clone();
        let commit_arc_clone = total_pads_committed_arc.clone();
        let sem_clone = semaphore.clone();

        join_set.spawn(async move {
            let permit = sem_clone.acquire_owned().await.expect("Semaphore closed");
            debug!(
                "ExecuteUploadTask[{}][Free]: Acquired permit. Writing pad index {}",
                key_for_task, pad_index
            );
            let write_result = pad_manager::write::write_chunk(
                &ma_storage,
                pad_info.address,
                &pad_info.key,
                &chunk,
                false, // is_new is false for Free
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
                Ok(_) => Ok(pad_index),
                Err(e) => {
                    error!(
                        "ExecuteUploadTask[{}][Free]: write_chunk failed for index {}: {}",
                        key_for_task, pad_index, e
                    );
                    Err(e)
                }
            }
        });
    }

    // Collect results for Free phase
    let mut successfully_updated_indices = Vec::new();
    let mut total_bytes_written_in_update: u64 = 0;
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok(pad_index)) => {
                successfully_updated_indices.push(pad_index);
                let chunk_size_for_progress = if pad_index < data_chunks.len() {
                    data_chunks[pad_index].len() as u64
                } else {
                    0
                };
                total_bytes_written_in_update += chunk_size_for_progress;

                // Emit incremental UploadProgress
                let current_total_bytes =
                    bytes_written_during_create + total_bytes_written_in_update;
                invoke_callback(
                    &mut *callback_arc.lock().await,
                    PutEvent::UploadProgress {
                        bytes_written: current_total_bytes,
                        total_bytes: total_bytes_expected,
                    },
                )
                .await?;
            }
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
                join_set.abort_all();
            }
            Err(je) => {
                error!("ExecuteUpload[{}][Free]: JoinError: {}", key_owned, je);
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

    // Batch update status and persist after successful Free phase
    if !successfully_updated_indices.is_empty() {
        debug!(
            "ExecuteUpload[{}][Free]: Batch updating status for {} pads...",
            key_owned,
            successfully_updated_indices.len()
        );
        let mut mis_guard = ma.master_index_storage.lock().await;
        let mut modified = false;
        if let Some(key_info) = mis_guard.index.get_mut(&key_owned) {
            for idx in successfully_updated_indices {
                if let Some(pi_mut) = key_info.pads.get_mut(idx) {
                    if pi_mut.status == PadUploadStatus::Free {
                        // Avoid double update
                        pi_mut.status = PadUploadStatus::Populated;
                        modified = true;
                    }
                }
            }
            if modified {
                key_info.modified = Utc::now();
            }
        }
        if modified {
            let network_choice = ma.storage.get_network_choice();
            let mis_data_to_write = mis_guard.clone();
            drop(mis_guard);
            if let Err(e) = write_local_index(&mis_data_to_write, network_choice).await {
                error!(
                    "ExecuteUpload[{}][Free]: Failed to persist state after batch update: {}. Continuing...",
                    key_owned, e
                );
            }
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
            // Use Arc/Mutex callback
            invoke_callback(&mut *callback_arc.lock().await, PutEvent::UploadFinished).await?;
            // Emit StoreComplete after UploadFinished
            invoke_callback(&mut *callback_arc.lock().await, PutEvent::StoreComplete).await?;
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
