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

        // --- Step 1: Initial Resource Check (Under Lock) ---
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
            (
                initial_pads_to_keep,
                initial_recycle_on_success,
                initial_needed_from_free,
                initial_pads_to_reserve,
            ) = self.acquire_resources_for_write(&mut mis_guard, &key_owned, data_size)?;
            debug!(
                "AllocateWrite[{}]: Releasing lock after initial check. Keep:{}, Recycle:{}, NeedFree:{}, Reserve:{}",
                key_owned, initial_pads_to_keep.len(), initial_recycle_on_success.len(), initial_needed_from_free, initial_pads_to_reserve
            );
        } // Lock released

        // --- Step 1.5: Take Pads from Free List (Under Lock) ---
        let pads_taken_from_free: Vec<PadInfo>;
        if initial_needed_from_free > 0 {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired to take {} pads from free list (available: {}).",
                key_owned,
                initial_needed_from_free,
                mis_guard.free_pads.len()
            );
            if mis_guard.free_pads.len() < initial_needed_from_free {
                // This should ideally not happen if acquire_resources_for_write was correct
                // and no other write interfered, but handle defensively.
                error!(
                    "AllocateWrite[{}]: CRITICAL: Not enough pads in free list ({}) when {} were expected. State inconsistency?",
                    key_owned, mis_guard.free_pads.len(), initial_needed_from_free
                );
                return Err(Error::InternalError(format!(
                    "Free list state mismatch for key {}",
                    key_owned
                )));
            }
            // Calculate length before mutable borrow
            let current_free_len = mis_guard.free_pads.len();
            let start_index = current_free_len - initial_needed_from_free;
            // Drain the required number of pads from the end of the free list
            pads_taken_from_free = mis_guard.free_pads.drain(start_index..).collect();
            debug!(
                "AllocateWrite[{}]: Took {} pads. Free list size now: {}. Releasing lock.",
                key_owned,
                pads_taken_from_free.len(),
                mis_guard.free_pads.len()
            );
            // Lock released when mis_guard goes out of scope
        } else {
            pads_taken_from_free = Vec::new();
        }

        let shared_callback = Arc::new(Mutex::new(callback.take()));

        // Create MPSC channel for reserved pads
        // Channel buffer size - can be tuned. 1 allows the first reserved pad to be sent immediately.
        let (pad_info_tx, pad_info_rx) = mpsc::channel::<Result<PadInfo, Error>>(1);

        // --- Step 2 & 4 Concurrently: Reserve New Pads & Perform Write ---

        // Clone necessary Arcs for the reservation task
        let res_key = key_owned.clone();
        let res_cb = shared_callback.clone();
        let res_storage = self.storage.clone();
        let res_mis = self.master_index_storage.clone();
        let res_tx = pad_info_tx; // Move sender to reservation task

        // Spawn reservation task
        let reservation_handle = tokio::spawn(async move {
            if initial_pads_to_reserve > 0 {
                debug!(
                    "AllocateWrite-ResTask[{}]: Starting reservation for {} pads...",
                    res_key, initial_pads_to_reserve
                );
                reserve_new_pads_and_collect(
                    &res_key,
                    initial_pads_to_reserve,
                    res_cb,
                    res_tx, // Pass the sender
                    res_storage,
                    res_mis,
                )
                .await
            } else {
                debug!("AllocateWrite-ResTask[{}]: No new pads needed.", res_key);
                Ok(())
                // Sender (res_tx) is dropped here implicitly, closing the channel correctly
            }
        });

        // Clone/Prepare necessary items for the write task
        let write_key = key_owned.clone();
        let write_cb = shared_callback.clone();
        let write_storage = self.storage.clone();
        let write_mis = self.master_index_storage.clone(); // Needed for final save
        let data_arc = Arc::new(data.to_vec()); // Clone data into Arc for tasks
        let write_initial_pads = initial_pads_to_keep.clone(); // Pads available immediately
        let write_pads_from_free = pads_taken_from_free; // Pads taken from free list
        let write_rx = pad_info_rx; // Pads from reservation channel

        // Spawn the actual concurrent write task using the standalone function
        let write_handle = tokio::spawn(async move {
            debug!(
                "AllocateWrite-WriteTask[{}]: Starting concurrent write processing...",
                write_key
            );
            // Call the standalone function from the concurrent module
            concurrent::perform_concurrent_write_standalone(
                &write_key,
                data_arc,
                write_initial_pads,
                write_pads_from_free, // Pass free pads
                write_rx,
                write_storage,
                write_mis,
                write_cb,
            )
            .await
        });

        // Await both tasks and handle potential JoinError first
        let (reservation_result_inner, write_result_inner) =
            match tokio::try_join!(reservation_handle, write_handle) {
                Ok((res_inner, write_inner)) => (res_inner, write_inner),
                Err(join_err) => {
                    error!(
                        "AllocateWrite[{}]: Task join error: {}",
                        key_owned, join_err
                    );
                    return Err(Error::from_join_error_msg(
                        &join_err,
                        "Task join error during allocate_and_write".to_string(),
                    ));
                }
            };

        // --- Handle Individual Task Results ---

        // Check reservation result (inner Result<Ok(()), Error>)
        match reservation_result_inner {
            Ok(_) => {} // Reservation task succeeded
            Err(e) => {
                // Reservation task completed but returned an internal error
                error!(
                    "AllocateWrite[{}]: Reservation task failed internally: {}",
                    key_owned, e
                );
                return Err(e);
            }
        };

        // Check write result (inner Result<Ok(pads), Error>)
        let successfully_written_pads = match write_result_inner {
            Ok(pads) => pads, // Write task succeeded and returned Ok(pads)
            Err(e) => {
                // Write task completed but returned an internal error
                error!(
                    "AllocateWrite[{}]: Write task failed internally: {}",
                    key_owned, e
                );
                return Err(e);
            }
        };

        // --- Both tasks succeeded internally ---
        info!(
            "AllocateWrite[{}]: Reservation and Write tasks completed successfully.",
            key_owned
        );

        // --- Step 3 (In-Memory Update) & Step 5 (Final Save) combined ---
        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for final index update and commit.",
                key_owned
            );

            // Use the list returned by the successful write task.
            let final_pads_list = successfully_written_pads;

            let final_pad_count = final_pads_list.len(); // For logging

            // Check if expected number of pads were used (rough check)
            let expected_pads = calculate_needed_pads(data_size, mis_guard.scratchpad_size)?;
            if final_pad_count != expected_pads {
                warn!(
                    "AllocateWrite[{}]: Final pad count ({}) mismatch expected ({}). Proceeding anyway.",
                    key_owned, final_pad_count, expected_pads
                );
            }

            // Update the index in memory
            let key_info = KeyStorageInfo {
                pads: final_pads_list, // Use the list determined above
                data_size,
                modified: chrono::Utc::now(),
            };
            mis_guard.index.insert(key_owned.clone(), key_info);

            // Handle recycling pads from shrinking updates
            if !initial_recycle_on_success.is_empty() {
                debug!(
                    "AllocateWrite[{}]: Recycling {} pads from successful update (final commit).",
                    key_owned,
                    initial_recycle_on_success.len()
                );
                mis_guard
                    .free_pads
                    .extend(initial_recycle_on_success.clone());
            }

            debug!(
                 "AllocateWrite[{}]: Final in-memory index update complete. Performing final save...",
                 key_owned
            );

            // Get MIS info for saving
            let (mis_addr, mis_key) = self.storage.get_master_index_info();
            // Drop guard before await on save
            drop(mis_guard);

            // Save the final master index state
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
                    // Invoke StoreComplete *after* final save
                    if let Err(cb_err) =
                        invoke_callback(&mut *final_callback_guard, PutEvent::StoreComplete).await
                    {
                        error!(
                            "AllocateWrite[{}]: Failed to invoke StoreComplete callback: {}",
                            key_owned, cb_err
                        );
                        // Continue even if callback fails, state is saved.
                    }
                    info!(
                        "AllocateWrite[{}]: Operation completed successfully.",
                        key_owned
                    );
                    Ok(())
                }
                Err(save_err) => {
                    error!(
                        "AllocateWrite[{}]: CRITICAL: Failed final save of master index after successful write: {}. State might be inconsistent.",
                        key_owned, save_err
                    );
                    // Data is written, but final index update failed.
                    Err(save_err)
                }
            }
        }
    }
}
