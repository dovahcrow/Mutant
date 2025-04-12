use super::PadManager;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::{KeyStorageInfo, MasterIndexStorage};
use crate::storage::storage_save_mis_from_arc_static; // Import static save function
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

// Helper function placed here or in util.rs
fn calculate_needed_pads(data_size: usize, scratchpad_size: usize) -> Result<usize, Error> {
    if scratchpad_size == 0 {
        error!("Scratchpad size is 0, cannot calculate needed pads.");
        return Err(Error::InternalError("Scratchpad size is zero".to_string()));
    }
    Ok((data_size + scratchpad_size - 1) / scratchpad_size)
}

// Import the function directly
use self::reservation::reserve_new_pads_and_collect;

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
            Vec<PadInfo>, // pads_to_keep (from existing entry)
            Vec<PadInfo>, // pads_to_recycle_on_success (from shrinking update)
            usize,        // needed_from_free (how many to take from free list later)
            usize,        // pads_to_reserve (how many to reserve if free list is insufficient)
        ),
        Error,
    > {
        let scratchpad_size = mis_guard.scratchpad_size;
        let needed_total_pads = calculate_needed_pads(data_size, scratchpad_size)?;

        debug!(
            "AcquireResources[{}]: data_size={}, scratchpad_size={}, needed_pads={}",
            key, data_size, scratchpad_size, needed_total_pads
        );

        let pads_to_keep: Vec<PadInfo>;
        let pads_to_recycle_on_success: Vec<PadInfo>;
        let needed_from_free: usize;
        let pads_to_reserve: usize;

        if let Some(existing_info) = mis_guard.index.get(key).cloned() {
            let old_num_pads = existing_info.pads.len();
            debug!(
                "AcquireResources[{}]: Update path. Existing pads: {}, Needed pads: {}",
                key, old_num_pads, needed_total_pads
            );

            if needed_total_pads <= old_num_pads {
                // Shrinking or same size
                pads_to_keep = existing_info.pads[0..needed_total_pads].to_vec();
                pads_to_recycle_on_success = existing_info.pads[needed_total_pads..].to_vec();
                needed_from_free = 0;
                pads_to_reserve = 0;
            } else {
                // Growing
                pads_to_keep = existing_info.pads; // Keep all existing
                let additional_pads_required = needed_total_pads - old_num_pads;
                pads_to_recycle_on_success = Vec::new(); // Not shrinking

                // Calculate how many are needed from free/reserve
                let available_free = mis_guard.free_pads.len();
                needed_from_free = std::cmp::min(additional_pads_required, available_free);
                pads_to_reserve = additional_pads_required - needed_from_free;
            }
        } else {
            // New key path
            debug!(
                "AcquireResources[{}]: New path. Needed pads: {}",
                key, needed_total_pads
            );
            pads_to_keep = Vec::new(); // No existing pads to keep
            pads_to_recycle_on_success = Vec::new(); // No existing pads to recycle

            let available_free = mis_guard.free_pads.len();
            needed_from_free = std::cmp::min(needed_total_pads, available_free);
            pads_to_reserve = needed_total_pads - needed_from_free;
        }

        debug!(
            "AcquireResources[{}]: Results: keep={}, recycle={}, need_from_free={}, reserve={}",
            key,
            pads_to_keep.len(),
            pads_to_recycle_on_success.len(),
            needed_from_free,
            pads_to_reserve,
        );

        Ok((
            pads_to_keep,
            pads_to_recycle_on_success,
            needed_from_free,
            pads_to_reserve,
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

        // Declare variables from Step 1 results needed later
        let initial_pads_to_keep: Vec<PadInfo>;
        let initial_recycle_on_success: Vec<PadInfo>;
        let initial_needed_from_free: usize;
        let initial_pads_to_reserve: usize;

        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for initial resource check",
                key_owned
            );
            // Use the read-only acquire function
            let (keep, recycle, needed_free, reserve) =
                self.acquire_resources_for_write(&mut mis_guard, &key_owned, data_size)?;

            // Assign results to outer scope variables
            initial_pads_to_keep = keep;
            initial_recycle_on_success = recycle;
            initial_needed_from_free = needed_free;
            initial_pads_to_reserve = reserve;

            debug!(
                "AllocateWrite[{}]: Releasing lock after initial resource check. Keep: {}, Recycle: {}, Need Free: {}, Reserve: {}",
                key_owned,
                initial_pads_to_keep.len(),
                initial_recycle_on_success.len(),
                initial_needed_from_free,
                initial_pads_to_reserve
            );
            // mis_guard is dropped here
        }

        let shared_callback = Arc::new(Mutex::new(callback.take()));
        let newly_reserved_pads_collector = Arc::new(Mutex::new(Vec::new())); // Renamed for clarity

        // --- Step 2: Reserve New Pads if Needed and Save State ---
        if initial_pads_to_reserve > 0 {
            debug!(
                "AllocateWrite[{}]: Starting reservation for {} pads...",
                key_owned, initial_pads_to_reserve
            );

            // Call the standalone function
            let reservation_result = reserve_new_pads_and_collect(
                &key_owned,
                initial_pads_to_reserve,
                shared_callback.clone(),
                newly_reserved_pads_collector.clone(),
                self.storage.clone(), // Pass Arc<Storage>
            )
            .await;

            match reservation_result {
                Ok(_) => {
                    // The collector now holds the reserved pads
                    let reserved_pads_guard = newly_reserved_pads_collector.lock().await;
                    let num_reserved = reserved_pads_guard.len();
                    debug!(
                        "AllocateWrite[{}]: Successfully reserved {} pads (collected). Saving intermediate state...",
                        key_owned,
                        num_reserved
                    );
                    // Important: Clone the pads before extending the free list
                    let reserved_pads_cloned = reserved_pads_guard.clone();
                    // Drop the guard *before* locking MIS
                    drop(reserved_pads_guard);

                    // Lock MIS, add reserved pads to free list, save MIS
                    {
                        let mut mis_guard = self.master_index_storage.lock().await;
                        debug!(
                            "AllocateWrite[{}]: Lock acquired for saving reserved pads to free list",
                             key_owned
                        );
                        // Use the cloned list
                        mis_guard.free_pads.extend(reserved_pads_cloned);
                        debug!(
                            "AllocateWrite[{}]: Added {} reserved pads to free list (new total: {}).",
                            key_owned,
                            num_reserved, // Use num_reserved here
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
                            // Attempt to revert: Remove the pads we just tried to add.
                            {
                                let mut mis_guard_revert = self.master_index_storage.lock().await;
                                // Assume they were added at the end, revert the last N
                                let _reserved_pads_guard =
                                    newly_reserved_pads_collector.lock().await; // Prefix with _
                                let revert_count = num_reserved;
                                if mis_guard_revert.free_pads.len() >= revert_count {
                                    let len = mis_guard_revert.free_pads.len();
                                    mis_guard_revert.free_pads.truncate(len - revert_count);
                                    debug!(
                                        "AllocateWrite[{}]: Reverted adding {} pads to free list.",
                                        key_owned, revert_count
                                    );
                                }
                            }
                            // Even though save failed, reservation likely succeeded ($$), so we don't return the collector pads
                            return Err(e);
                        }
                        debug!(
                            "AllocateWrite[{}]: Intermediate save successful after reservation.",
                            key_owned
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "AllocateWrite[{}]: Reservation failed: {}. Cleaning up resources.",
                        key_owned, e
                    );
                    // Reservation failed, no new pads were added to free list.
                    // Cleanup: If the initial check took pads from free list (it shouldn't with new logic),
                    // return them. Check 'initial_taken_free' (which should be empty now)
                    // We don't need to lock MIS here as we are just checking a variable.
                    // **NOTE:** The 'initial_taken_free' concept is removed from the new acquire_resources_for_write.
                    // There's nothing to return here from the free list perspective for reservation failure.
                    return Err(e);
                }
            }
        } else {
            debug!("AllocateWrite[{}]: No new pads needed.", key_owned);
        }

        // --- Step 3: Acquire Final Resources (Pads to Keep + Pads from Free List) ---
        let (final_pads_to_use, final_taken_free, final_recycle_on_success_confirmed) = {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for final resource acquisition",
                key_owned
            );

            // RECALCULATE needed pads based on current state (data size and kept pads)
            let scratchpad_size = mis_guard.scratchpad_size;
            let total_needed = calculate_needed_pads(data_size, scratchpad_size)?;
            let needed_now = total_needed.saturating_sub(initial_pads_to_keep.len());
            debug!(
                "AllocateWrite[{}]: Recalculated final needs: total={}, kept={}, need_now={}",
                key_owned,
                total_needed,
                initial_pads_to_keep.len(),
                needed_now
            );

            // Take pads from free list based on the RECALCULATED need
            let mut actual_taken_free: Vec<PadInfo> = Vec::new();
            if needed_now > 0 {
                // Check if we actually need to take pads now
                let num_available = mis_guard.free_pads.len();
                let num_to_take = std::cmp::min(needed_now, num_available);

                if num_to_take < needed_now {
                    // This error condition remains relevant
                    error!(
                        "AllocateWrite[{}]: Logic Error! Expected {} pads from free list, but only {} available.",
                        key_owned, needed_now, num_to_take
                    );
                    // Attempt cleanup: Return newly reserved pads? This is tricky as they *should* be in free list now.
                    // Best to just error out.
                    return Err(Error::InternalError(
                        "Insufficient free pads during final acquisition".to_string(),
                    ));
                }
                debug!(
                    "AllocateWrite[{}]: Taking {} pads from free list (available: {}).",
                    key_owned, num_to_take, num_available
                );
                actual_taken_free = mis_guard.free_pads.drain(..num_to_take).collect();
            } else {
                debug!(
                    "AllocateWrite[{}]: No pads needed from free list in final step.",
                    key_owned
                );
            }

            // Combine kept pads (from Step 1) and newly taken free pads
            let mut final_pads = initial_pads_to_keep; // Use variable from Step 1
            final_pads.extend(actual_taken_free.iter().cloned());

            debug!(
                "AllocateWrite[{}]: Releasing lock after final resource acquisition. Pads to use: {}, Taken free: {}, Recycle: {}",
                key_owned,
                final_pads.len(),
                actual_taken_free.len(),
                initial_recycle_on_success.len() // Use variable from Step 1
            );

            // Confirm the recycle list from Step 1
            (final_pads, actual_taken_free, initial_recycle_on_success)
        };

        // --- Step 4: Perform Concurrent Write ---
        // Use the final determined list of pads
        debug!("AllocateWrite[{}]: Starting final write...", key_owned);
        let write_result = self
            .perform_concurrent_write(
                &key_owned,
                data,
                final_pads_to_use.clone(), // Use the finally acquired pads
                // Pass an empty receiver if reservation happened (pads added to free list),
                // otherwise pass the collector if reservation *didn't* happen but pads were reserved (shouldn't occur with this logic)
                // Pass an empty receiver always now, as reserved pads go directly to free list.
                {
                    let (tx, rx) = mpsc::channel(1); // Create a dummy channel
                    drop(tx);
                    rx
                },
                // Pass the collector for newly reserved pads (concurrent write needs it, though it might be empty)
                // Let's pass an empty collector now, as reserved pads are handled before write
                Arc::new(Mutex::new(Vec::new())), // Empty collector
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
                // Return pads intended for writing (final_pads_to_use)
                // These came from initial_pads_to_keep + actual_taken_free
                let _num_to_return = final_pads_to_use.len(); // Prefix with _
                                                              // We cannot just extend free_pads with final_pads_to_use directly,
                                                              // as it contains duplicates if pads were kept.
                                                              // We need to return the ones *taken* from the free list:
                let num_returned_free = final_taken_free.len(); // Log length before move
                mis_guard.free_pads.extend(final_taken_free);
                // And we need to acknowledge that the 'kept' pads remain associated with the key (even if write failed)
                // The index wasn't updated, so the old KeyStorageInfo (with 'kept' pads) is still there.
                // If the key didn't exist before, there are no 'kept' pads.

                // Do NOT return initial_recycle_on_success pads here, as they were never removed from index.

                debug!(
                    "AllocateWrite[{}]: Cleanup complete. Returned {} pads taken from free list. Kept pads remain with key.",
                    key_owned,
                    num_returned_free, // Use the stored length
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
            // Use the confirmed recycle list from Step 3
            if !final_recycle_on_success_confirmed.is_empty() {
                debug!(
                    "AllocateWrite[{}]: Recycling {} pads from successful update.",
                    key_owned,
                    final_recycle_on_success_confirmed.len()
                );
                mis_guard
                    .free_pads
                    .extend(final_recycle_on_success_confirmed);
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
