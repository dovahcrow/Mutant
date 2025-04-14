use super::PadInfoAlias;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::Storage as BaseStorage;
use crate::utils::retry::retry_operation;
use autonomi::SecretKey;
use log::{debug, error, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub(super) const WRITE_TASK_CONCURRENCY: usize = 100;

// --- Standalone Concurrent Write Logic ---

/// Performs concurrent writes using immediately available pads and pads streamed from a channel.
/// Returns a list of PadInfo for all pads successfully written to
pub(super) async fn perform_concurrent_write_standalone(
    key: &str,
    data_arc: Arc<Vec<u8>>,
    initial_pads: Vec<PadInfoAlias>,
    pads_from_free: Vec<PadInfoAlias>,
    mut reserved_pad_rx: mpsc::Receiver<Result<PadInfoAlias, Error>>,
    storage: Arc<BaseStorage>,
    master_index_storage: Arc<Mutex<MasterIndexStorage>>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<Vec<PadInfoAlias>, Error> {
    let total_size = data_arc.len();
    let key_owned = key.to_string();

    let successfully_written_pads_map: Arc<Mutex<HashMap<usize, PadInfoAlias>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let scratchpad_size = {
        let mis_guard = master_index_storage.lock().await;
        if mis_guard.scratchpad_size == 0 {
            error!(
                "PerformWrite[{}]: Scratchpad size is 0, cannot write.",
                key_owned
            );
            return Err(Error::InternalError("Scratchpad size is zero".to_string()));
        }
        mis_guard.scratchpad_size
    };

    let mut join_set: JoinSet<Result<(usize, PadInfoAlias), Error>> = JoinSet::new();
    let semaphore = Arc::new(Semaphore::new(WRITE_TASK_CONCURRENCY));
    let total_bytes_uploaded = Arc::new(Mutex::new(0u64));
    let total_pads_committed = Arc::new(Mutex::new(0u64));
    let mut bytes_assigned: usize = 0;
    let mut current_pad_index: usize = 0;
    let mut first_error: Option<Error> = None;

    if total_size > 0 {
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(
            &mut *cb_guard,
            PutEvent::StartingUpload {
                total_bytes: total_size as u64,
            },
        )
        .await?;
    }

    // --- Determine Total Expected Pads ---
    // This is an approximation for now. A more accurate count should be passed down.
    let total_pads_expected = (total_size + scratchpad_size - 1) / scratchpad_size;
    debug!(
        "PerformWrite[{}]: Estimated total pads expected: {}",
        key_owned, total_pads_expected
    );

    // --- Process Initial/Reused Pads ---
    debug!(
        "PerformWrite[{}]: Processing {} immediately available pads.",
        key_owned,
        initial_pads.len()
    );
    for (pad_address, pad_key_bytes) in initial_pads {
        let chunk_start = bytes_assigned;
        let chunk_end = std::cmp::min(chunk_start + scratchpad_size, total_size);
        let chunk_len = chunk_end - chunk_start;

        if chunk_len == 0 && chunk_start >= total_size {
            debug!(
                "PerformWrite[{}]: Reached end of data with initial pads.",
                key_owned
            );
            break;
        }

        let task_chunk_start = chunk_start;
        let task_chunk_end = chunk_end;
        bytes_assigned = chunk_end;
        let current_pad_info = (pad_address.clone(), pad_key_bytes.clone());

        debug!(
            "PerformWrite[{}]: Spawning writer for Initial (Reused) Pad Index {} ({} bytes)",
            key_owned, current_pad_index, chunk_len
        );
        spawn_write_task_standalone(
            &mut join_set,
            semaphore.clone(),
            callback_arc.clone(),
            total_bytes_uploaded.clone(),
            total_pads_committed.clone(),
            total_pads_expected,
            total_size as u64,
            storage.clone(),
            current_pad_index,
            pad_key_bytes,
            data_arc.clone(),
            task_chunk_start,
            task_chunk_end,
            scratchpad_size,
            current_pad_info,
            key_owned.clone(),
            false, // Not newly reserved
        );
        current_pad_index += 1;
    }

    // --- Process Pads Taken from Free List ---
    debug!(
        "PerformWrite[{}]: Processing {} pads taken from free list.",
        key_owned,
        pads_from_free.len()
    );
    for (pad_address, pad_key_bytes) in pads_from_free {
        let chunk_start = bytes_assigned;
        let chunk_end = std::cmp::min(chunk_start + scratchpad_size, total_size);
        let chunk_len = chunk_end - chunk_start;

        if chunk_len == 0 && chunk_start >= total_size {
            debug!(
                "PerformWrite[{}]: Reached end of data with free pads.",
                key_owned
            );
            break;
        }

        let task_chunk_start = chunk_start;
        let task_chunk_end = chunk_end;
        bytes_assigned = chunk_end;
        let current_pad_info = (pad_address.clone(), pad_key_bytes.clone());

        debug!(
            "PerformWrite[{}]: Spawning writer for Free Pad Index {} ({} bytes)",
            key_owned, current_pad_index, chunk_len
        );
        spawn_write_task_standalone(
            &mut join_set,
            semaphore.clone(),
            callback_arc.clone(),
            total_bytes_uploaded.clone(),
            total_pads_committed.clone(),
            total_pads_expected,
            total_size as u64,
            storage.clone(),
            current_pad_index,
            pad_key_bytes,
            data_arc.clone(),
            task_chunk_start,
            task_chunk_end,
            scratchpad_size,
            current_pad_info,
            key_owned.clone(),
            false, // Not newly reserved
        );
        current_pad_index += 1;
    }

    debug!(
        "PerformWrite[{}]: Finished spawning initial/free writers. Bytes assigned: {}. Waiting for reserved pads...",
        key_owned, bytes_assigned
    );

    while let Some(pad_result) = reserved_pad_rx.recv().await {
        match pad_result {
            Ok(pad_info) => {
                let (pad_address, pad_key_bytes) = pad_info;
                let current_pad_info = (pad_address.clone(), pad_key_bytes.clone());

                let chunk_start = bytes_assigned;
                let chunk_end = std::cmp::min(chunk_start + scratchpad_size, total_size);
                let chunk_len = chunk_end - chunk_start;

                if chunk_len > 0 {
                    let task_chunk_start = chunk_start;
                    let task_chunk_end = chunk_end;
                    bytes_assigned = chunk_end;

                    debug!(
                        "PerformWrite[{}]: Spawning writer for Reserved Pad Index {} ({} bytes)",
                        key_owned, current_pad_index, chunk_len
                    );
                    spawn_write_task_standalone(
                        &mut join_set,
                        semaphore.clone(),
                        callback_arc.clone(),
                        total_bytes_uploaded.clone(),
                        total_pads_committed.clone(),
                        total_pads_expected,
                        total_size as u64,
                        storage.clone(),
                        current_pad_index,
                        pad_key_bytes,
                        data_arc.clone(),
                        task_chunk_start,
                        task_chunk_end,
                        scratchpad_size,
                        current_pad_info,
                        key_owned.clone(),
                        true, // Newly reserved
                    );
                } else if chunk_start >= total_size {
                    debug!(
                        "PerformWrite[{}]: Received Reserved Pad Index {} but all data already assigned.",
                        key_owned, current_pad_index
                    );
                } else {
                    warn!(
                        "PerformWrite[{}]: Received Reserved Pad Index {} unexpectedly resulted in zero chunk length.",
                        key_owned, current_pad_index
                    );
                }
                current_pad_index += 1;
            }
            Err(e) => {
                error!(
                    "PerformWrite[{}]: Received reservation error from channel: {}. Halting writes.",
                    key_owned, e
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
                break;
            }
        }
    }
    debug!(
        "PerformWrite[{}]: Pad receiver channel closed. Total pads considered: {}. Bytes assigned: {}. Waiting for {} tasks...",
        key_owned, current_pad_index, bytes_assigned, join_set.len()
    );

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok(task_result) => match task_result {
                Ok((pad_index, pad_info)) => {
                    let mut map_guard = successfully_written_pads_map.lock().await;
                    map_guard.insert(pad_index, pad_info);
                    drop(map_guard);
                }
                Err(e) => {
                    error!("Write Sub-Task[{}]: Failed: {}", key_owned, e);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            },
            Err(join_error) => {
                error!("Write Task Join[{}]: JoinError: {}", key_owned, join_error);
                if first_error.is_none() {
                    first_error = Some(Error::from_join_error_msg(
                        &join_error,
                        "Write sub-task join error".to_string(),
                    ));
                }
            }
        }
    }
    debug!(
        "PerformWrite[{}]: All write tasks finished processing.",
        key_owned
    );

    if let Some(e) = first_error {
        error!("PerformWrite[{}]: Completed with error: {}", key_owned, e);
        Err(e)
    } else {
        let final_bytes_uploaded = *total_bytes_uploaded.lock().await;
        if final_bytes_uploaded != total_size as u64 {
            error!(
                "PerformWrite[{}]: Byte count mismatch! Expected: {}, Reported Uploaded: {}",
                key_owned, total_size, final_bytes_uploaded
            );
            Err(Error::InternalError(
                "Write completion byte count mismatch".to_string(),
            ))
        } else {
            let final_pads_map = Arc::try_unwrap(successfully_written_pads_map)
                .map_err(|_| {
                    Error::InternalError(
                        "Failed to unwrap Arc for successfully written pads map".to_string(),
                    )
                })?
                .into_inner();

            let mut sorted_pads: Vec<(usize, PadInfoAlias)> = final_pads_map.into_iter().collect();
            sorted_pads.sort_by_key(|&(index, _)| index);

            let final_pads: Vec<PadInfoAlias> = sorted_pads
                .into_iter()
                .map(|(_, pad_info)| pad_info)
                .collect();

            debug!(
                "PerformWrite[{}]: Completed successfully. {} bytes written across {} pads.",
                key_owned,
                final_bytes_uploaded,
                final_pads.len()
            );

            // Send UploadFinished event
            debug!(
                "PerformWrite[{}]: Emitting UploadFinished event.",
                key_owned
            );
            {
                let mut cb_guard = callback_arc.lock().await;
                invoke_callback(&mut *cb_guard, PutEvent::UploadFinished).await?;
            }

            Ok(final_pads)
        }
    }
}

/// Spawns a single write task. (Standalone function version)
#[allow(clippy::too_many_arguments)]
fn spawn_write_task_standalone(
    join_set: &mut JoinSet<Result<(usize, PadInfoAlias), Error>>,
    semaphore: Arc<Semaphore>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
    total_bytes_uploaded_arc: Arc<Mutex<u64>>,
    total_pads_committed_arc: Arc<Mutex<u64>>,
    total_pads_expected: usize,
    total_size_overall: u64,
    storage: Arc<BaseStorage>,
    pad_index: usize,
    pad_key_bytes: Vec<u8>,
    data_arc: Arc<Vec<u8>>,
    chunk_start: usize,
    chunk_end: usize,
    _scratchpad_size: usize,
    pad_info_for_task: PadInfoAlias,
    key_str: String,
    is_newly_reserved: bool,
) {
    // Capture necessary Arcs and variables for the async block
    let task_storage = storage.clone();
    let task_pad_key_bytes = pad_key_bytes;
    let task_data_arc = data_arc.clone();
    let task_chunk_start = chunk_start;
    let task_chunk_end = chunk_end;
    let task_pad_info = pad_info_for_task;
    // --- Variables needed for internal progress reporting ---
    let task_key_str = key_str.clone();
    let task_pad_index = pad_index;
    let task_callback_arc = callback_arc.clone();
    let task_total_bytes_uploaded_arc = total_bytes_uploaded_arc.clone();
    let task_bytes_in_chunk = (chunk_end - chunk_start) as u64;
    let task_total_size_overall = total_size_overall;
    let task_is_newly_reserved = is_newly_reserved;
    let task_total_pads_committed_arc = total_pads_committed_arc.clone();
    let task_total_pads_expected = total_pads_expected;
    // --- End variables needed ---

    join_set.spawn(async move {
        // Acquire semaphore permit
        let permit = semaphore
            .acquire()
            .await
            .map_err(|_| Error::InternalError("Semaphore closed unexpectedly".to_string()))?;

        // Decode the SecretKey
        let key_array: [u8; 32] = task_pad_key_bytes.as_slice().try_into().map_err(|_| {
            Error::InvalidInput(format!(
                "Pad key has incorrect length. Expected 32 bytes, got {}",
                task_pad_key_bytes.len()
            ))
        })?;
        let owner_key = SecretKey::from_bytes(key_array)
            .map_err(|e| Error::InvalidInput(format!("Invalid pad key bytes: {}", e)))?;

        // Prepare data chunk
        let chunk_data = &task_data_arc[task_chunk_start..task_chunk_end];

        debug!(
            "Task[{}][Pad {}]: Starting verified write via retry_operation ({} bytes)",
            task_key_str,
            task_pad_index,
            chunk_data.len()
        );

        // --- Perform the verified write using retry_operation --- //
        // The storage function now handles internal progress reporting
        retry_operation(
            &format!("Verified write for {} Pad {}", task_key_str, task_pad_index),
            // Closure captures necessary variables and calls the _with_progress storage func
            || async {
                // Note: We capture Arcs and clone them *inside* the closure if needed by the function
                // But storage::update_scratchpad_internal_static_with_progress takes refs to Arcs,
                // so capturing the Arcs directly is fine.
                let storage_client = task_storage.get_client().await?;
                crate::storage::update_scratchpad_internal_static_with_progress(
                    storage_client,
                    &owner_key,
                    chunk_data,
                    0, // Assuming content_type 0 for now
                    &task_key_str,
                    task_pad_index,
                    &task_callback_arc,
                    &task_total_bytes_uploaded_arc,
                    task_bytes_in_chunk,
                    task_total_size_overall,
                    task_is_newly_reserved,
                    &task_total_pads_committed_arc,
                    task_total_pads_expected,
                )
                .await
            },
            |_e: &Error| true, // Retry on any error for now
        )
        .await?;
        // --- Verified write successful --- //

        // --- OLD Progress reporting logic removed --- //
        // UploadProgress and ScratchpadCommitComplete events are now emitted
        // *inside* update_scratchpad_internal_static_with_progress.

        drop(permit); // Release semaphore permit explicitly before returning

        debug!(
            "Task[{}][Pad {}]: Write task completed successfully. Returning index and info.",
            task_key_str, task_pad_index
        );

        Ok((task_pad_index, task_pad_info))
    });
}
