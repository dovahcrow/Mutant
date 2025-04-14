use super::PadManager;
use crate::cache::write_local_index;
use crate::error::Error;
use crate::events::PutCallback;
use crate::mutant::data_structures::{KeyStorageInfo, MasterIndexStorage};
use crate::storage::{network, Storage};
use autonomi::{ScratchpadAddress, SecretKey};
use chrono;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard; // Added

mod concurrent;
mod reservation;

pub(crate) type PadInfoAlias = (ScratchpadAddress, Vec<u8>);

/// Writes a single data chunk to a specified scratchpad.
///
/// This function acts as a wrapper around the storage layer's create/update operations,
/// handling the choice between creating a new pad or updating an existing one.
/// It relies on the underlying storage functions to handle confirmation loops.
///
/// # Arguments
/// * `storage` - Reference to the storage interface.
/// * `address` - The address of the scratchpad to write to.
/// * `key_bytes` - The encryption key bytes for the scratchpad.
/// * `data_chunk` - The chunk of data to write.
/// * `is_new_pad` - Boolean indicating whether to create a new pad (`true`) or update an existing one (`false`).
///
/// # Returns
/// Returns `Ok(())` on successful write and confirmation, otherwise a `crate::error::Error`.
pub async fn write_chunk(
    storage: &Storage,
    address: ScratchpadAddress,
    key_bytes: &[u8],
    data_chunk: &[u8],
    is_new_pad: bool,
) -> Result<(), Error> {
    debug!(
        "write_chunk: Address={}, Key=<{} bytes>, Chunk=<{} bytes>, IsNew={}",
        address,
        key_bytes.len(),
        data_chunk.len(),
        is_new_pad
    );

    let key_array: [u8; 32] = key_bytes.try_into().map_err(|_| {
        Error::InvalidInput(format!(
            "Invalid secret key byte length: expected 32, got {}",
            key_bytes.len()
        ))
    })?;
    let secret_key = SecretKey::from_bytes(key_array)
        .map_err(|e| Error::InvalidInput(format!("Invalid secret key bytes: {}", e)))?;

    let client = storage.get_client().await?;

    if is_new_pad {
        let wallet = storage.wallet();
        let payment_option = autonomi::client::payment::PaymentOption::from(wallet);
        let created_address =
            network::create_scratchpad_static(client, &secret_key, data_chunk, 0, payment_option)
                .await?;
        debug!(
            "write_chunk: Pad creation successful for {}",
            created_address
        );
        Ok(())
    } else {
        network::update_scratchpad_internal_static(client, &secret_key, data_chunk, 0).await
    }
}

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
    /// * `Vec<PadInfoAlias>`: Pads to use immediately (kept from old + taken from free).
    /// * `usize`: Number of *new* pads that need to be reserved.
    /// * `Vec<PadInfoAlias>`: Pads taken from the free list (to be returned on failure).
    /// * `Vec<PadInfoAlias>`: Old pads to be recycled *on success* (only relevant for updates).
    fn acquire_resources_for_write<'a>(
        &self,
        mis_guard: &mut MutexGuard<'a, MasterIndexStorage>,
        key: &str,
        data_size: usize,
    ) -> Result<
        (
            Vec<PadInfoAlias>, // pads_to_keep (from existing entry)
            Vec<PadInfoAlias>, // pads_to_recycle_on_success (from shrinking update)
            usize,             // needed_from_free (how many to take from free list later)
            usize,             // pads_to_reserve (how many to reserve if free list is insufficient)
        ),
        Error,
    > {
        let scratchpad_size = mis_guard.scratchpad_size;
        let needed_total_pads = calculate_needed_pads(data_size, scratchpad_size)?;

        debug!(
            "AcquireResources[{}]: data_size={}, scratchpad_size={}, needed_pads={}",
            key, data_size, scratchpad_size, needed_total_pads
        );

        let pads_to_keep: Vec<PadInfoAlias>;
        let pads_to_recycle_on_success: Vec<PadInfoAlias>;
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
                pads_to_keep = existing_info.pads[0..needed_total_pads]
                    .iter()
                    .map(|pi| (pi.address, pi.key.clone()))
                    .collect();
                pads_to_recycle_on_success = existing_info.pads[needed_total_pads..]
                    .iter()
                    .map(|pi| (pi.address, pi.key.clone()))
                    .collect();
                needed_from_free = 0;
                pads_to_reserve = 0;
            } else {
                // Growing
                pads_to_keep = existing_info
                    .pads
                    .iter()
                    .map(|pi| (pi.address, pi.key.clone()))
                    .collect();
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
        let initial_pads_to_keep: Vec<PadInfoAlias>;
        let initial_recycle_on_success: Vec<PadInfoAlias>;
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
        let pads_taken_from_free: Vec<PadInfoAlias>;
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
        let (pad_info_tx, pad_info_rx) = mpsc::channel::<Result<PadInfoAlias, Error>>(1);

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

        // Spawn the write task
        let write_handle = tokio::spawn(async move {
            concurrent::perform_concurrent_write_standalone(
                &write_key,
                data_arc,
                write_initial_pads,
                write_pads_from_free,
                write_rx, // Pass receiver to write task
                write_storage,
                write_mis, // Pass MIS for final update
                write_cb,
            )
            .await
        });

        // --- Step 5: Wait for Results and Finalize --- //
        let reservation_result = reservation_handle.await.map_err(|e| {
            Error::from_join_error_msg(&e, format!("Reservation task failed for {}", key_owned))
        })?;

        let write_result = write_handle.await.map_err(|e| {
            Error::from_join_error_msg(&e, format!("Write task failed for {}", key_owned))
        })?;

        if let Err(e) = reservation_result {
            error!(
                "AllocateWrite[{}]: Reservation task returned error: {}",
                key_owned, e
            );
            return Err(e);
        }

        let written_pads = match write_result {
            Ok(pads) => {
                debug!(
                    "AllocateWrite[{}]: Write task successful. {} pads written/reused.",
                    key_owned,
                    pads.len()
                );
                pads
            }
            Err(e) => {
                error!(
                    "AllocateWrite[{}]: Write task returned error: {}",
                    key_owned, e
                );
                return Err(e);
            }
        };

        // --- Step 6: Update Master Index (Under Lock) ---
        {
            let mut mis_guard = self.master_index_storage.lock().await;
            debug!(
                "AllocateWrite[{}]: Lock acquired for final MIS update",
                key_owned
            );

            // Add/Update the key entry
            let final_pads_info: Vec<crate::mutant::data_structures::PadInfo> = written_pads
                .into_iter()
                .map(|(addr, key)| crate::mutant::data_structures::PadInfo {
                    address: addr,
                    key,
                    // Status should reflect the outcome, but write task doesn't provide it.
                    // Assuming Populated for now, but this is incorrect for resumability.
                    status: crate::mutant::data_structures::PadUploadStatus::Populated,
                    // is_new is also unknown here.
                    is_new: false, // Placeholder!
                })
                .collect();

            mis_guard.index.insert(
                key_owned.clone(),
                KeyStorageInfo {
                    pads: final_pads_info, // Use the converted Vec<PadInfo>
                    data_size,
                    data_checksum: String::new(), // Placeholder!
                    modified: chrono::Utc::now(),
                },
            );
            debug!("AllocateWrite[{}]: Updated index entry.", key_owned);

            // Add any pads that were kept but originally marked for recycling back to free list
            if !initial_recycle_on_success.is_empty() {
                debug!(
                    "AllocateWrite[{}]: Adding {} originally recycled pads back to free list.",
                    key_owned,
                    initial_recycle_on_success.len()
                );
                mis_guard.free_pads.extend(initial_recycle_on_success);
            }

            debug!(
                "AllocateWrite[{}]: Releasing lock after final MIS update.",
                key_owned
            );
        } // Lock released

        // --- Step 7: Write to Local Cache (Instead of Remote) ---
        info!(
            "AllocateWrite[{}]: Persisting updated index to local cache...",
            key_owned
        );
        // Call write_local_index AFTER the lock is released
        let network = self.storage.get_network_choice(); // Get network choice from storage
        write_local_index(&*self.master_index_storage.lock().await, network).await?; // Pass network

        info!(
            "PadManager::AllocateWrite[{}]: Operation completed successfully.",
            key_owned
        );
        Ok(())
    }
}
