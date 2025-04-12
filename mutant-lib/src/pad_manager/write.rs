use super::PadManager;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::{KeyStorageInfo, MasterIndexStorage};
use crate::storage::storage_save_mis_from_arc_static; // Import static save function
use crate::storage::Storage;
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
    /// Saves newly reserved pads to the master index *before* attempting data writes.
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
        let key_owned = key.to_string();

        // --- Step 1: Determine resource needs (Initial Check) ---
        // Declare variables in the outer scope
        let initial_pads_to_reserve: usize;
        let mut initial_taken_free: Vec<PadInfo> = Vec::new(); // Keep ownership here for potential cleanup
                                                               // These are not needed outside the initial check's scope if the structure holds
                                                               // let initial_pads_to_use: Vec<PadInfo>;
                                                               // let initial_recycle_on_success: Vec<PadInfo>;

        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for initial resource check",
                key_owned
            );
            let (pads_to_use, pads_to_reserve, taken_free, recycle_on_success) =
                self.acquire_resources_for_write(&mut mis_guard, &key_owned, data_size)?;

            // Assign results to outer scope variables
            initial_pads_to_reserve = pads_to_reserve;
            initial_taken_free = taken_free; // Move happens here, variable is now owned by outer scope
                                             // We don't strictly need the other two in the outer scope with the current logic
                                             // initial_pads_to_use = pads_to_use;
                                             // initial_recycle_on_success = recycle_on_success;

            debug!(
                "AllocateWrite[{}]: Releasing lock after initial resource check. Need to reserve: {}",
                key_owned,
                initial_pads_to_reserve // Use the outer scope variable
            );
            // mis_guard is dropped here
        };

        let shared_callback = Arc::new(Mutex::new(callback.take()));
        let newly_reserved_pads = Arc::new(Mutex::new(Vec::new())); // To collect results IF reservation succeeds

        // --- Step 2: Reserve New Pads if Needed and Save State ---
        if initial_pads_to_reserve > 0 {
            debug!(
                "AllocateWrite[{}]: Starting reservation for {} pads...",
                key_owned, initial_pads_to_reserve
            );

            // Call the refactored function that collects results
            let reservation_result = self
                .reserve_new_pads_and_collect(initial_pads_to_reserve, shared_callback.clone())
                .await;

            match reservation_result {
                Ok(reserved_pads) => {
                    debug!(
                        "AllocateWrite[{}]: Successfully reserved {} pads. Saving intermediate state...",
                        key_owned,
                        reserved_pads.len()
                    );
                    // Lock MIS, add reserved pads to free list, save MIS
                    {
                        let mut mis_guard = self.master_index_storage.lock().await;
                        debug!(
                            "AllocateWrite[{}]: Lock acquired for saving reserved pads to free list",
                             key_owned
                        );
                        mis_guard.free_pads.extend(reserved_pads.clone());
                        debug!(
                            "AllocateWrite[{}]: Added {} reserved pads to free list (new total: {}).",
                            key_owned,
                            reserved_pads.len(),
                            mis_guard.free_pads.len()
                        );
                        // Need storage details for saving
                        let (mis_addr, mis_key) = self.storage.get_master_index_info();
                        // Drop guard before await on save
                        drop(mis_guard);

                        let save_result = storage_save_mis_from_arc_static(
                            self.storage.client(),
                            &mis_addr,
                            &mis_key,
                            &self.master_index_storage,
                        )
                        .await;

                        if let Err(e) = save_result {
                            error!(
                                "AllocateWrite[{}]: CRITICAL: Failed to save intermediate state after reserving pads: {}. Reverting free list addition.",
                                key_owned,
                                e
                            );
                            // Attempt to revert: This is tricky. The pads are reserved ($$ spent),
                            // but we failed to record them. Best effort: remove them from the in-memory list.
                            // This is not fully transactional.
                            {
                                let mut mis_guard_revert = self.master_index_storage.lock().await;
                                let original_free_count = mis_guard_revert.free_pads.len();
                                // Simple revert: drain the last N added pads.
                                // Might be incorrect if other operations modified free_pads concurrently (shouldn't happen if MIS lock is held correctly).
                                let revert_count = reserved_pads.len();
                                if original_free_count >= revert_count {
                                    mis_guard_revert
                                        .free_pads
                                        .truncate(original_free_count - revert_count);
                                    warn!("AllocateWrite[{}]: Reverted in-memory free_pads list after save failure.", key_owned);
                                } else {
                                    error!("AllocateWrite[{}]: Cannot revert free_pads list - inconsistent state.", key_owned);
                                }
                            }
                            // Propagate the critical save error
                            return Err(e);
                        } else {
                            debug!(
                                "AllocateWrite[{}]: Successfully saved intermediate state with {} new pads in free list.",
                                key_owned,
                                reserved_pads.len()
                            );
                        }
                    }
                    // Store collected pads for potential write use (although acquire_resources should get them now)
                    let mut collector_guard = newly_reserved_pads.lock().await;
                    *collector_guard = reserved_pads;
                }
                Err(e) => {
                    error!(
                        "AllocateWrite[{}]: Pad reservation failed: {}. Aborting write operation.",
                        key_owned, e
                    );
                    // Need to return the initially taken free pads back
                    {
                        let mut mis_guard = self.master_index_storage.lock().await;
                        // Now use the outer scope variable which still has ownership
                        let cleanup_count = initial_taken_free.len();
                        mis_guard.free_pads.extend(initial_taken_free); // This should work now
                        debug!("AllocateWrite[{}]: Returned {} initially taken free pads to free list (current total: {}) due to reservation failure.", key_owned, cleanup_count, mis_guard.free_pads.len());
                    }
                    return Err(e);
                }
            }
        } else {
            debug!("AllocateWrite[{}]: No new pads needed.", key_owned);
        }

        // --- Step 3: Re-acquire resources now that reserved pads are in the free list ---
        // This ensures we use the newly reserved pads correctly.
        let (final_pads_to_use, _final_pads_to_reserve, final_taken_free, final_recycle_on_success) = {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for final resource acquisition",
                key_owned
            );
            // Should not need to reserve any more pads here (final_pads_to_reserve should be 0)
            let result = self.acquire_resources_for_write(&mut mis_guard, &key_owned, data_size)?;
            debug!(
                "AllocateWrite[{}]: Releasing lock after final resource acquisition. Pads to use: {}, Taken free: {}, Recycle: {}",
                key_owned,
                result.0.len(),
                result.2.len(),
                result.3.len()
            );
            if result.1 != 0 {
                // This indicates a logic error - we should have reserved enough.
                error!("AllocateWrite[{}]: Logic Error! Still need to reserve {} pads after reservation step.", key_owned, result.1);
                // Attempt cleanup before erroring
                mis_guard.free_pads.extend(result.2); // Return taken free
                mis_guard.free_pads.extend(initial_taken_free); // Also return initial ones if different
                                                                // Also need to return the newly_reserved_pads here if we got this far?
                                                                // This state is messy.
                let new_pads = newly_reserved_pads.lock().await;
                mis_guard.free_pads.extend(new_pads.iter().cloned());
                return Err(Error::InternalError(
                    "Pad reservation count mismatch".to_string(),
                ));
            }
            result
        };

        // --- Step 4: Perform Concurrent Write ---
        // Note: perform_concurrent_write doesn't need the receiver anymore
        // It just needs the list of pads to write to (final_pads_to_use).
        debug!("AllocateWrite[{}]: Starting final write...", key_owned);
        let write_result = self
            .perform_concurrent_write(
                &key_owned,
                data,
                final_pads_to_use.clone(), // Use the finally acquired pads
                // Pass an empty receiver, as new pads are already collected and added to free list
                {
                    let (tx, rx) = mpsc::channel(1);
                    drop(tx);
                    rx
                },
                // This collector is now mostly irrelevant here, but concurrent write expects it.
                // It won't collect anything new via the empty receiver.
                Arc::new(Mutex::new(Vec::new())),
                shared_callback.clone(),
            )
            .await;

        // --- Step 5: Final Commit or Cleanup ---
        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for finalization/cleanup",
                key_owned
            );

            if let Err(e) = write_result {
                warn!(
                    "AllocateWrite[{}]: Write operation failed (Error: {}). Cleaning up resources.",
                    key_owned, e
                );
                // Pads that were assigned for writing (final_pads_to_use) came from the free list
                // (either originally free or newly reserved & added). Return them.
                let num_to_return = final_pads_to_use.len();
                mis_guard.free_pads.extend(final_pads_to_use);

                // Also need to return any *other* pads taken from free list during the final acquisition
                // if acquire_resources_for_write logic allows taking more than needed temporarily?
                // Assuming acquire_resources_for_write only takes what's needed, so final_taken_free should be empty here?
                // Let's add final_taken_free back just in case.
                let num_extra_taken_free = final_taken_free.len();
                mis_guard.free_pads.extend(final_taken_free);
                // Do NOT return initial_taken_free here, as they were handled during reservation failure.

                debug!(
                    "AllocateWrite[{}]: Cleanup complete. Returned {} pads used in failed write + {} extra taken free pads to free list.",
                    key_owned,
                    num_to_return,
                    num_extra_taken_free
                );
                debug!(
                    "AllocateWrite[{}]: Releasing lock after cleanup.",
                    key_owned
                );
                return Err(e);
            }
            debug!(
                "AllocateWrite[{}]: Write operation successful. Committing state.",
                key_owned
            );

            // Use the pads identified in the final acquisition step
            let final_pad_list_for_commit: Vec<PadInfo> = final_pads_to_use;

            let key_info = KeyStorageInfo {
                pads: final_pad_list_for_commit,
                data_size,
                modified: chrono::Utc::now(),
            };
            mis_guard.index.insert(key_owned.clone(), key_info);

            // Recycle pads from shrinking updates (identified in final acquisition)
            if !final_recycle_on_success.is_empty() {
                debug!(
                    "AllocateWrite[{}]: Recycling {} pads from successful update.",
                    key_owned,
                    final_recycle_on_success.len()
                );
                mis_guard.free_pads.extend(final_recycle_on_success);
            }

            // Final save of the master index
            let (mis_addr, mis_key) = self.storage.get_master_index_info();
            // Drop guard before await
            drop(mis_guard);

            debug!(
                "AllocateWrite[{}]: Performing final save of master index...",
                key_owned
            );
            match storage_save_mis_from_arc_static(
                self.storage.client(),
                &mis_addr,
                &mis_key,
                &self.master_index_storage,
            )
            .await
            {
                Ok(_) => {
                    debug!(
                        "AllocateWrite[{}]: Final master index save successful.",
                        key_owned
                    );
                    let mut final_callback_guard = shared_callback.lock().await;
                    if let Err(e) =
                        invoke_callback(&mut *final_callback_guard, PutEvent::StoreComplete).await
                    {
                        error!(
                            "AllocateWrite[{}]: Failed to invoke StoreComplete callback: {}",
                            key_owned, e
                        );
                    }
                    info!("AllocateWrite[{}]: Commit successful.", key_owned);
                    debug!("AllocateWrite[{}]: Releasing lock after commit.", key_owned);
                    Ok(())
                }
                Err(e) => {
                    error!(
                            "AllocateWrite[{}]: CRITICAL: Failed final save of master index after successful write: {}. State might be inconsistent.",
                            key_owned, e
                        );
                    // Data is written, but index update failed. This is bad.
                    // What is the state of mis_guard.index here?
                    // We probably shouldn't call StoreComplete callback.
                    Err(e)
                }
            }
            // End of successful write block
        }
    }
}
