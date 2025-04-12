use super::super::PadManager;
use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::storage::Storage as BaseStorage;
use crate::utils::retry::retry_operation;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::WRITE_TASK_CONCURRENCY;

impl PadManager {
    /// Performs concurrent writes using immediately available pads and pads streamed from a channel.
    pub(super) async fn perform_concurrent_write(
        &self,
        key: &str,
        data: &[u8],
        initial_pads: Vec<PadInfo>,
        mut reserved_pad_rx: mpsc::Receiver<Result<PadInfo, Error>>,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
    ) -> Result<(), Error> {
        let total_size = data.len();
        let key_owned = key.to_string();

        let scratchpad_size = {
            let mis_guard = self.master_index_storage.lock().await;
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
        let data_arc = Arc::new(data.to_vec());
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

            debug!(
                "PerformWrite[{}]: Spawning writer for Initial Pad Index {} ({} bytes)",
                key_owned, current_pad_index, chunk_len
            );
            self.spawn_write_task(
                &mut join_set,
                semaphore.clone(),
                callback_arc.clone(),
                total_bytes_uploaded.clone(),
                key_owned.clone(),
                current_pad_index,
                pad_address,
                pad_key_bytes,
                data_arc.clone(),
                task_chunk_start,
                task_chunk_end,
                scratchpad_size,
                total_size as u64,
            );
            current_pad_index += 1;
        }
        debug!(
            "PerformWrite[{}]: Finished spawning initial writers. Bytes assigned: {}. Waiting for reserved pads...",
            key_owned, bytes_assigned
        );

        while let Some(pad_result) = reserved_pad_rx.recv().await {
            match pad_result {
                Ok(pad_info) => {
                    let (pad_address, pad_key_bytes) = pad_info;

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
                        self.spawn_write_task(
                            &mut join_set,
                            semaphore.clone(),
                            callback_arc.clone(),
                            total_bytes_uploaded.clone(),
                            key_owned.clone(),
                            current_pad_index,
                            pad_address,
                            pad_key_bytes,
                            data_arc.clone(),
                            task_chunk_start,
                            task_chunk_end,
                            scratchpad_size,
                            total_size as u64,
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
                debug!(
                    "PerformWrite[{}]: Completed successfully. {} bytes written.",
                    key_owned, final_bytes
                );
                Ok(())
            }
        }
    }

    fn spawn_write_task(
        &self,
        join_set: &mut JoinSet<Result<(), Error>>,
        semaphore: Arc<Semaphore>,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
        total_bytes_uploaded: Arc<Mutex<u64>>,
        key_for_task: String,
        pad_index: usize,
        pad_address: ScratchpadAddress,
        pad_key_bytes: Vec<u8>,
        data_arc: Arc<Vec<u8>>,
        chunk_start: usize,
        chunk_end: usize,
        scratchpad_size: usize,
        total_size_overall: u64,
    ) {
        let task_storage = self.storage.clone();
        let task_semaphore = semaphore;
        let task_callback = callback_arc;
        let task_bytes_uploaded = total_bytes_uploaded;

        join_set.spawn(async move {
            let permit = task_semaphore
                .acquire_owned()
                .await
                .map_err(|_| Error::InternalError("Semaphore closed unexpectedly".to_string()))?;

            let pad_secret_key = {
                let key_array: [u8; 32] = match pad_key_bytes.as_slice().try_into() {
                    Ok(arr) => arr,
                    Err(_) => {
                        return Err(Error::InternalError(format!(
                            "WriteTask[{}]: Pad#{} key byte conversion failed (len {})",
                            key_for_task,
                            pad_index,
                            pad_key_bytes.len()
                        )));
                    }
                };
                match SecretKey::from_bytes(key_array) {
                    Ok(key) => key,
                    Err(e) => {
                        return Err(Error::InternalError(format!(
                            "WriteTask[{}]: Pad#{} SecretKey creation failed: {}",
                            key_for_task, pad_index, e
                        )));
                    }
                }
            };

            let data_chunk_slice = &data_arc[chunk_start..chunk_end];

            let write_result = Self::perform_single_pad_write_static(
                task_storage,
                pad_index,
                pad_address,
                pad_secret_key,
                data_chunk_slice,
                scratchpad_size,
                task_callback,
                task_bytes_uploaded,
                total_size_overall,
            )
            .await;

            drop(permit);
            write_result
        });
    }

    async fn perform_single_pad_write_static(
        storage: Arc<BaseStorage>,
        pad_log_index: usize,
        pad_address: ScratchpadAddress,
        pad_key: SecretKey,
        data_chunk: &[u8],
        expected_pad_size: usize,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
        total_bytes_uploaded_arc: Arc<Mutex<u64>>,
        total_bytes_overall: u64,
    ) -> Result<(), Error> {
        let chunk_len = data_chunk.len();
        debug!(
            "SinglePadWrite[Pad #{}]: Starting write for {} bytes to {:?}. Arc ptr: {:?}",
            pad_log_index,
            chunk_len,
            pad_address,
            Arc::as_ptr(&total_bytes_uploaded_arc)
        );

        if chunk_len > expected_pad_size {
            error!(
                "SinglePadWrite[Pad #{}]: Chunk length {} exceeds expected pad size {}. Aborting.",
                pad_log_index, chunk_len, expected_pad_size
            );
            return Err(Error::InternalError(format!(
                "Data chunk too large for pad {}",
                pad_log_index
            )));
        }

        let pad_content = data_chunk.to_vec();

        const WRITE_CONTENT_TYPE: u64 = 0;
        let update_desc = format!("Update pad {} ({})", pad_log_index, pad_address);

        retry_operation(
            &update_desc,
            || {
                let storage_inner = storage.clone();
                let pad_key_inner = pad_key.clone();
                let content_inner = pad_content.clone();

                async move {
                    let storage_client = storage_inner.client();
                    let res = crate::storage::update_scratchpad_internal_static(
                        storage_client,
                        &pad_key_inner,
                        &content_inner,
                        WRITE_CONTENT_TYPE,
                    )
                    .await;
                    Ok(res?)
                }
            },
            |_e: &Error| true,
        )
        .await?;

        let new_total_uploaded = {
            let mut guard = total_bytes_uploaded_arc.lock().await;
            *guard += chunk_len as u64;
            debug!(
                "SinglePadWrite[Pad #{}]: Updated total_bytes_uploaded by {} to {}. Arc ptr: {:?}",
                pad_log_index,
                chunk_len,
                *guard,
                Arc::as_ptr(&total_bytes_uploaded_arc)
            );
            *guard
        };

        let event = PutEvent::UploadProgress {
            bytes_written: new_total_uploaded,
            total_bytes: total_bytes_overall,
        };
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(&mut *cb_guard, event).await?;

        debug!(
            "SinglePadWrite[Pad #{}]: Successfully updated pad {} and reported progress ({} bytes written).",
            pad_log_index, pad_address, chunk_len
        );

        Ok(())
    }
}
