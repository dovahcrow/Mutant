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
    /// Helper function to perform concurrent writes using immediately available pads and newly reserved pads streamed from a channel.
    pub(super) async fn perform_concurrent_write(
        &self,
        key: &str,
        data: &[u8],
        pads_to_use_immediately: Vec<PadInfo>,
        mut new_pad_receiver: mpsc::Receiver<Result<PadInfo, Error>>,
        newly_reserved_pads_collector: Arc<Mutex<Vec<PadInfo>>>,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
    ) -> Result<(), Error> {
        let total_size = data.len();
        let key_owned = key.to_string();

        if total_size == 0 && pads_to_use_immediately.is_empty() {
            debug!(
                 "PerformWrite[{}]: Skipping initial write for 0 byte data and 0 immediate pads. Waiting for new pads...",
                 key_owned
             );
        }

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
            pads_to_use_immediately.len()
        );
        for (pad_address, pad_key_bytes) in pads_to_use_immediately {
            let chunk_start = bytes_assigned;
            let chunk_end = std::cmp::min(chunk_start + scratchpad_size, total_size);
            let chunk_len = chunk_end - chunk_start;

            if chunk_len == 0 && chunk_start >= total_size {
                break;
            }

            let data_chunk = data[chunk_start..chunk_end].to_vec();
            bytes_assigned = chunk_end;

            debug!(
                "PerformWrite[{}]: Spawning writer for Immediate Pad Index {} ({} bytes)",
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
                data_chunk,
                scratchpad_size,
                total_size,
            );
            current_pad_index += 1;
        }
        debug!(
            "PerformWrite[{}]: Finished spawning writers for immediate pads. Bytes assigned: {}",
            key_owned, bytes_assigned
        );

        debug!(
            "PerformWrite[{}]: Waiting for newly reserved pads from channel...",
            key_owned
        );
        while let Some(pad_result) = new_pad_receiver.recv().await {
            match pad_result {
                Ok(pad_info) => {
                    {
                        let mut collector_guard = newly_reserved_pads_collector.lock().await;
                        collector_guard.push(pad_info.clone());
                    }

                    let (pad_address, pad_key_bytes) = pad_info;

                    let chunk_start = bytes_assigned;
                    let chunk_end = std::cmp::min(chunk_start + scratchpad_size, total_size);
                    let chunk_len = chunk_end - chunk_start;

                    if chunk_len > 0 {
                        let data_chunk = data[chunk_start..chunk_end].to_vec();
                        bytes_assigned = chunk_end;

                        debug!(
                            "PerformWrite[{}]: Spawning writer for New Pad Index {} ({} bytes)",
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
                            data_chunk,
                            scratchpad_size,
                            total_size,
                        );
                    } else if chunk_start >= total_size {
                        debug!(
                            "PerformWrite[{}]: Received New Pad Index {} but all data already assigned.",
                            key_owned, current_pad_index
                        );
                    } else {
                        warn!(
                            "PerformWrite[{}]: Received New Pad Index {} resulting in zero chunk length unexpectedly.",
                            key_owned, current_pad_index
                        );
                    }
                    current_pad_index += 1;
                }
                Err(e) => {
                    error!(
                        "PerformWrite[{}]: Received reservation error from channel: {}",
                        key_owned, e
                    );
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        debug!("PerformWrite[{}]: Pad receiver channel closed. Total pads processed (immediate + new): {}. Bytes assigned: {}", key_owned, current_pad_index, bytes_assigned);

        debug!(
            "PerformWrite[{}]: All pads processed or channel closed. Waiting for {} write tasks...",
            key_owned,
            join_set.len()
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
                    error!("Write Task[{}]: JoinError: {}", key_owned, join_error);
                    if first_error.is_none() {
                        first_error = Some(Error::from_join_error_msg(
                            &join_error,
                            "Write sub-task join error".to_string(),
                        ));
                    }
                }
            }
        }
        debug!("PerformWrite[{}]: All write tasks finished.", key_owned);

        if let Some(e) = first_error {
            Err(e)
        } else {
            let final_bytes = *total_bytes_uploaded.lock().await;
            if final_bytes != total_size as u64 {
                error!(
                    "PerformWrite[{}]: Byte count mismatch after write completion. Expected: {}, Reported: {}",
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
        data_chunk: Vec<u8>,
        scratchpad_size: usize,
        total_size: usize,
    ) {
        let task_storage = self.storage.clone();
        let task_semaphore = semaphore;
        let task_callback = callback_arc;
        let task_bytes_uploaded = total_bytes_uploaded;
        let task_address = pad_address;
        let task_key_bytes = pad_key_bytes;

        join_set.spawn(async move {
            let permit = task_semaphore
                .acquire_owned()
                .await
                .expect("Write semaphore closed");

            let key_array: [u8; 32] = match task_key_bytes.as_slice().try_into() {
                Ok(arr) => arr,
                Err(_) => {
                    error!(
                        "PerformWrite[{}]: Invalid key length for pad index {}",
                        key_for_task, pad_index
                    );
                    return Err(Error::InternalError("Invalid key length".to_string()));
                }
            };
            let task_key = match SecretKey::from_bytes(key_array) {
                Ok(k) => k,
                Err(e) => {
                    error!(
                        "PerformWrite[{}]: Failed to reconstruct key for pad index {}: {}",
                        key_for_task, pad_index, e
                    );
                    return Err(Error::InternalError(format!(
                        "Failed to reconstruct key: {}",
                        e
                    )));
                }
            };

            let write_res = Self::perform_single_pad_write_static(
                task_storage,
                pad_index,
                task_address,
                task_key,
                &data_chunk,
                scratchpad_size,
                task_callback,
                task_bytes_uploaded,
                total_size as u64,
            )
            .await;

            drop(permit);
            write_res
        });
    }

    /// Static helper to perform the write operation for a single pad chunk.
    /// Assumes the pad should be overwritten with the provided chunk data, padded with zeros.
    async fn perform_single_pad_write_static(
        storage: Arc<BaseStorage>,
        pad_log_index: usize, // Just for logging
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
            "PadWriteStatic[{}]: Writing {} bytes to pad {}",
            pad_log_index, chunk_len, pad_address
        );

        if chunk_len > expected_pad_size {
            error!(
                "PadWriteStatic[{}]: Chunk length {} exceeds expected pad size {}. Aborting.",
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
            *guard
        };

        let event = PutEvent::UploadProgress {
            bytes_written: new_total_uploaded,
            total_bytes: total_bytes_overall,
        };
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(&mut *cb_guard, event).await?;

        debug!(
            "PadWriteStatic[{}]: Successfully updated pad {} and reported progress ({} bytes written).",
            pad_log_index, pad_address, chunk_len
        );

        Ok(())
    }
}
