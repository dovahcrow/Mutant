use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::Storage as BaseStorage;
use crate::utils::retry::retry_operation;
use autonomi::SecretKey;
use log::{debug, error, warn};
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
    initial_pads: Vec<PadInfo>,
    pads_from_free: Vec<PadInfo>,
    mut reserved_pad_rx: mpsc::Receiver<Result<PadInfo, Error>>,
    storage: Arc<BaseStorage>,
    master_index_storage: Arc<Mutex<MasterIndexStorage>>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<Vec<PadInfo>, Error> {
    let total_size = data_arc.len();
    let key_owned = key.to_string();

    let successfully_written_pads: Arc<Mutex<Vec<PadInfo>>> = Arc::new(Mutex::new(Vec::new()));

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

    let mut join_set = JoinSet::new();
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
            successfully_written_pads.clone(),
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
            successfully_written_pads.clone(),
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
                        successfully_written_pads.clone(),
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
            Ok(task_result) => {
                if let Err(e) = task_result {
                    error!("Write Sub-Task[{}]: Failed: {}", key_owned, e);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
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
            let final_pads = Arc::try_unwrap(successfully_written_pads)
                .map_err(|_| {
                    Error::InternalError(
                        "Failed to unwrap Arc for successfully written pads".to_string(),
                    )
                })?
                .into_inner();

            debug!(
                "PerformWrite[{}]: Completed successfully. {} bytes written across {} pads.",
                key_owned,
                final_bytes_uploaded,
                final_pads.len()
            );
            Ok(final_pads)
        }
    }
}

/// Spawns a single write task. (Standalone function version)
#[allow(clippy::too_many_arguments)]
fn spawn_write_task_standalone(
    join_set: &mut JoinSet<Result<(), Error>>,
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
    successfully_written_pads: Arc<Mutex<Vec<PadInfo>>>,
    pad_info_for_task: PadInfo,
    key_str: String,
    is_newly_reserved: bool,
) {
    join_set.spawn(async move {
        let semaphore_clone = semaphore.clone();
        let callback_arc_clone = callback_arc.clone();
        let total_bytes_uploaded_clone = total_bytes_uploaded_arc.clone();
        let total_pads_committed_clone = total_pads_committed_arc.clone();
        let storage_clone = storage.clone();
        let successfully_written_pads_clone = successfully_written_pads.clone();
        let task_pad_info = pad_info_for_task.clone();
        let permit = semaphore_clone.acquire_owned().await.unwrap();

        let pad_key = {
            let key_array: [u8; 32] = match pad_key_bytes.as_slice().try_into() {
                Ok(arr) => arr,
                Err(_) => {
                    error!(
                        "PadWrite[{}]: Invalid key bytes slice length for pad index {}",
                        key_str, pad_index
                    );
                    return Err(Error::InternalError(
                        "Pad key bytes have incorrect length".to_string(),
                    ));
                }
            };
            match SecretKey::from_bytes(key_array) {
                Ok(k) => k,
                Err(e) => {
                    error!(
                        "PadWrite[{}]: Failed to create SecretKey from bytes for pad index {}: {}",
                        key_str, pad_index, e
                    );
                    return Err(Error::InternalError(format!(
                        "Failed to create SecretKey: {}",
                        e
                    )));
                }
            }
        };

        let data_to_write = data_arc[chunk_start..chunk_end].to_vec();
        let bytes_in_chunk = data_to_write.len() as u64;

        // --- Perform Upload ---
        let upload_result = retry_operation(
            &format!("PadWrite[{}]: Upload Pad {}", key_str, pad_index),
            || async {
                // --- Actual Network Call ---
                let data_slice = &data_to_write;
                let client = storage_clone.get_client().await?;
                crate::storage::update_scratchpad_internal_static(client, &pad_key, data_slice, 0)
                    .await
            },
            |e: &Error| {
                warn!(
                    "PadWrite[{}]: Retrying upload for pad #{} due to error: {}",
                    key_str, pad_index, e
                );
                true
            },
        )
        .await;

        drop(permit);

        if upload_result.is_ok() {
            let current_total_uploaded = {
                let mut uploaded_guard = total_bytes_uploaded_clone.lock().await;
                *uploaded_guard += bytes_in_chunk;
                *uploaded_guard
            };

            {
                let mut cb_guard = callback_arc_clone.lock().await;
                invoke_callback(
                    &mut *cb_guard,
                    PutEvent::UploadProgress {
                        bytes_written: current_total_uploaded,
                        total_bytes: total_size_overall,
                    },
                )
                .await?;
            }

            // --- Commit Progress (only for newly reserved pads) ---
            if is_newly_reserved {
                let mut total_pads_committed = total_pads_committed_clone.lock().await;
                *total_pads_committed += 1;
                let current_committed = *total_pads_committed;
                // Use total_pads_expected which reflects the number of pads needed for the *data size*
                let total_for_commit_event = total_pads_expected as u64;
                drop(total_pads_committed); // Release lock before callback

                debug!(
                    "Task[{}][Pad {}]: ScratchpadCommitComplete (newly reserved): Pad {} / {}",
                    key_str, pad_index, current_committed, total_for_commit_event
                );
                let mut cb_guard = callback_arc_clone.lock().await;
                invoke_callback(
                    &mut *cb_guard,
                    PutEvent::ScratchpadCommitComplete {
                        index: current_committed.saturating_sub(1), // event uses 0-based index
                        total: total_for_commit_event,
                    },
                )
                .await?;
            } else {
                // Still log success for reused/free pads, but don't send commit event
                debug!(
                    "Task[{}][Pad {}]: Successfully wrote to reused/free pad.",
                    key_str, pad_index
                );
            }

            let mut pads_guard = successfully_written_pads_clone.lock().await;
            pads_guard.push(task_pad_info);
        }

        upload_result
    });
}
