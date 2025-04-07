use super::PadManager;
use crate::anthill::data_structures::{KeyStorageInfo, MasterIndexStorage};
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use autonomi::ScratchpadAddress;
use chrono;
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;

mod concurrent;
mod reservation;

pub(crate) const WRITE_TASK_CONCURRENCY: usize = 20;

pub(crate) type PadInfo = (ScratchpadAddress, Vec<u8>);

impl PadManager {
    /// Acquires the necessary pad resources (reused, free, new) for a write/update.
    ///
    /// This function **must** be called while holding the lock on MasterIndexStorage.
    /// It modifies `mis_guard.free_pads` by removing the pads designated for use.
    ///
    /// # Returns
    /// A tuple containing:
    /// * `Vec<PadInfo>`: Pads to use immediately (kept from old + taken from free).
    /// * `usize`: Number of *new* pads that need to be reserved.
    /// * `Vec<PadInfo>`: Pads taken from the free list (to be returned on failure).
    /// * `Vec<PadInfo>`: Old pads to be recycled *on success* (only relevant for updates).
    fn acquire_resources_for_write<'a>(
        &self,
        mis_guard: &mut MutexGuard<'a, MasterIndexStorage>,
        key: &str,
        data_size: usize,
    ) -> Result<
        (
            Vec<PadInfo>, // pads_to_use_immediately (kept/reused + from_free)
            usize,        // pads_to_reserve
            Vec<PadInfo>, // pads_taken_from_free (for failure cleanup)
            Vec<PadInfo>, // pads_to_recycle_on_success (from shrinking update)
        ),
        Error,
    > {
        let scratchpad_size = mis_guard.scratchpad_size;
        if scratchpad_size == 0 {
            error!("Scratchpad size is 0, cannot allocate pads.");
            return Err(Error::InternalError("Scratchpad size is zero".to_string()));
        }

        let needed_pads = (data_size + scratchpad_size - 1) / scratchpad_size;
        debug!(
            "AcquireResources[{}]: data_size={}, scratchpad_size={}, needed_pads={}",
            key, data_size, scratchpad_size, needed_pads
        );

        let mut pads_to_use_immediately: Vec<PadInfo> = Vec::new();
        #[allow(unused_assignments)]
        let mut pads_to_reserve: usize = 0;
        let mut pads_taken_from_free: Vec<PadInfo> = Vec::new();
        let mut pads_to_recycle_on_success: Vec<PadInfo> = Vec::new();

        if let Some(existing_info) = mis_guard.index.get(key).cloned() {
            let old_num_pads = existing_info.pads.len();
            debug!(
                "AcquireResources[{}]: Update path. Existing pads: {}, Needed pads: {}",
                key, old_num_pads, needed_pads
            );

            if needed_pads <= old_num_pads {
                pads_to_use_immediately = existing_info.pads[0..needed_pads].to_vec();
                pads_to_recycle_on_success = existing_info.pads[needed_pads..].to_vec();
                pads_to_reserve = 0;
            } else {
                pads_to_use_immediately = existing_info.pads;
                let additional_pads_required = needed_pads - old_num_pads;

                let num_from_free =
                    std::cmp::min(additional_pads_required, mis_guard.free_pads.len());
                pads_taken_from_free = mis_guard.free_pads.drain(..num_from_free).collect();
                pads_to_use_immediately.extend(pads_taken_from_free.iter().cloned());

                pads_to_reserve = additional_pads_required - num_from_free;
            }
        } else {
            debug!(
                "AcquireResources[{}]: New path. Needed pads: {}",
                key, needed_pads
            );
            let num_from_free = std::cmp::min(needed_pads, mis_guard.free_pads.len());
            if num_from_free > 0 {
                pads_taken_from_free = mis_guard.free_pads.drain(..num_from_free).collect();
                pads_to_use_immediately.extend(pads_taken_from_free.iter().cloned());
            }
            pads_to_reserve = needed_pads - num_from_free;
        }

        debug!(
            "AcquireResources[{}]: Results: use_immediately={}, reserve={}, taken_free={}, recycle_on_success={}",
            key,
            pads_to_use_immediately.len(),
            pads_to_reserve,
            pads_taken_from_free.len(),
            pads_to_recycle_on_success.len()
        );

        Ok((
            pads_to_use_immediately,
            pads_to_reserve,
            pads_taken_from_free,
            pads_to_recycle_on_success,
        ))
    }

    /// Allocates pads (reusing free ones first) and writes data concurrently.
    /// Handles both new keys and updates.
    pub async fn allocate_and_write(
        &self,
        key: &str,
        data: &[u8],
        mut callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        info!(
            "PadManager::AllocateWrite[{}]: Starting for {} bytes",
            key,
            data.len()
        );
        let data_size = data.len();

        let (
            pads_to_use_immediately,
            pads_to_reserve,
            pads_taken_from_free,
            pads_to_recycle_on_success,
        ) = {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for resource acquisition",
                key
            );
            let result = self.acquire_resources_for_write(&mut mis_guard, key, data_size)?;
            debug!(
                "AllocateWrite[{}]: Releasing lock after resource acquisition",
                key
            );
            result
        };

        let mut operation_error: Option<Error> = None;
        let shared_callback = Arc::new(Mutex::new(callback.take()));
        let newly_reserved_pads_collector = Arc::new(Mutex::new(Vec::new()));
        let mut opt_pad_receiver: Option<mpsc::Receiver<Result<PadInfo, Error>>> = None;

        let outer_result: Result<(), Error> = async {
            if pads_to_reserve > 0 {
                debug!(
                    "AllocateWrite[{}]: Starting reservation for {} pads...",
                    key, pads_to_reserve
                );
                let (pad_receiver, _join_handle) = self
                    .reserve_new_pads(pads_to_reserve, shared_callback.clone())
                    .await?;
                opt_pad_receiver = Some(pad_receiver);
                debug!(
                    "AllocateWrite[{}]: Reservation background task started.",
                    key
                );
            }

            let write_pad_receiver = match opt_pad_receiver.take() {
                Some(rx) => rx,
                None => {
                    let (tx, rx) = mpsc::channel(1);
                    drop(tx);
                    rx
                }
            };

            debug!("AllocateWrite[{}]: Starting write...", key);
            let write_result = self
                .perform_concurrent_write(
                    key,
                    data,
                    pads_to_use_immediately.clone(),
                    write_pad_receiver,
                    newly_reserved_pads_collector.clone(),
                    shared_callback.clone(),
                )
                .await;

            if let Err(e) = write_result {
                error!("AllocateWrite[{}]: Writing failed: {}", key, e);
                return Err(e);
            }

            debug!("AllocateWrite[{}]: Writing successful.", key);
            Ok(())
        }
        .await;

        if let Err(e) = outer_result {
            operation_error = Some(e);
        }

        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for finalization/cleanup",
                key
            );

            let collected_new_pads = newly_reserved_pads_collector.lock().await;

            if let Some(err) = operation_error {
                warn!(
                    "AllocateWrite[{}]: Operation failed (Error: {}). Cleaning up resources.",
                    key, err
                );
                let num_taken_free = pads_taken_from_free.len();
                let num_newly_reserved = collected_new_pads.len();

                mis_guard.free_pads.extend(pads_taken_from_free);
                mis_guard
                    .free_pads
                    .extend(collected_new_pads.iter().cloned());
                debug!(
                    "AllocateWrite[{}]: Cleanup complete. Returning {} pads to free list.",
                    key,
                    num_taken_free + num_newly_reserved
                );
                debug!("AllocateWrite[{}]: Releasing lock after cleanup.", key);
                return Err(err);
            } else {
                debug!(
                    "AllocateWrite[{}]: Operation successful. Committing state.",
                    key
                );

                let final_pad_list_for_commit: Vec<PadInfo> = pads_to_use_immediately
                    .iter()
                    .cloned()
                    .chain(collected_new_pads.iter().cloned())
                    .collect();

                let key_info = KeyStorageInfo {
                    pads: final_pad_list_for_commit,
                    data_size,
                    modified: chrono::Utc::now(),
                };
                mis_guard.index.insert(key.to_string(), key_info);

                if !pads_to_recycle_on_success.is_empty() {
                    debug!(
                        "AllocateWrite[{}]: Recycling {} pads from successful update.",
                        key,
                        pads_to_recycle_on_success.len()
                    );
                    mis_guard.free_pads.extend(pads_to_recycle_on_success);
                }

                let mut final_callback_guard = shared_callback.lock().await;
                if let Err(e) =
                    invoke_callback(&mut *final_callback_guard, PutEvent::StoreComplete).await
                {
                    error!(
                        "AllocateWrite[{}]: Failed to invoke StoreComplete callback: {}",
                        key, e
                    );
                }

                info!("AllocateWrite[{}]: Commit successful.", key);
                debug!("AllocateWrite[{}]: Releasing lock after commit.", key);
                Ok(())
            }
        }
    }
}
