use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::update_scratchpad_internal_static;
use crate::storage::Storage as BaseStorage;
use crate::utils::retry::retry_operation;
use autonomi::SecretKey;
use log::{debug, error, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub(super) const WRITE_TASK_CONCURRENCY: usize = 20;

// --- Standalone Concurrent Write Logic ---

/// Performs concurrent writes using immediately available pads and pads streamed from a channel.
/// Returns a list of PadInfo for all pads successfully written to.
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
        let final_bytes = *total_bytes_uploaded.lock().await;
        if final_bytes != total_size as u64 {
            error!(
                "PerformWrite[{}]: Byte count mismatch! Expected: {}, Reported: {}",
                key_owned, total_size, final_bytes
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
                final_bytes,
                final_pads.len()
            );
            Ok(final_pads)
        }
    }
}

/// Spawns a single write task. (Standalone function version)
fn spawn_write_task_standalone(
    join_set: &mut JoinSet<Result<(), Error>>,
    semaphore: Arc<Semaphore>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
    total_bytes_uploaded: Arc<Mutex<u64>>,
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
) {
    join_set.spawn(async move {
        let _permit = semaphore
            .acquire_owned()
            .await
            .map_err(|_| Error::InternalError("Semaphore closed unexpectedly".to_string()))?;

        let pad_secret_key = {
            let key_array: [u8; 32] = match pad_key_bytes.as_slice().try_into() {
                Ok(arr) => arr,
                Err(_) => {
                    return Err(Error::InternalError(
                        "Pad key bytes have incorrect length".to_string(),
                    ))
                }
            };
            SecretKey::from_bytes(key_array)
                .map_err(|e| Error::InternalError(format!("Failed to create SecretKey: {}", e)))?
        };

        let data_chunk = data_arc
            .get(chunk_start..chunk_end)
            .ok_or_else(|| Error::InternalError("Data chunk slicing failed".to_string()))?;

        let data_to_write = data_chunk.to_vec();
        let bytes_in_chunk = data_to_write.len() as u64;

        let write_result = retry_operation(
            &format!("Write Pad #{}", pad_index),
            || {
                update_scratchpad_internal_static(
                    storage.client(),
                    &pad_secret_key,
                    &data_to_write,
                    0,
                )
            },
            |e| {
                warn!(
                    "Retrying scratchpad write for pad #{} due to error: {}",
                    pad_index, e
                );
                true
            },
        )
        .await;

        if write_result.is_ok() {
            let current_total_bytes = {
                let mut total_guard = total_bytes_uploaded.lock().await;
                *total_guard += bytes_in_chunk;
                *total_guard
            };
            {
                let mut cb_guard = callback_arc.lock().await;
                invoke_callback(
                    &mut *cb_guard,
                    PutEvent::UploadProgress {
                        bytes_written: current_total_bytes,
                        total_bytes: total_size_overall,
                    },
                )
                .await?;
            }

            let mut pads_guard = successfully_written_pads.lock().await;
            pads_guard.push(pad_info_for_task);
        }

        write_result
    });
}
