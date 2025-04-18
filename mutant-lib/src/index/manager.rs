use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::AutonomiNetworkAdapter;
use crate::storage::manager::DefaultStorageManager;
use crate::types::{KeyDetails, StorageStats};
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Default implementation of the `IndexManager` trait.
/// Manages the state of the index, including key info, free pads, and pending verification pads.
pub struct DefaultIndexManager {
    state: Mutex<MasterIndex>,
    storage_manager: Arc<DefaultStorageManager>,
    network_adapter: Arc<AutonomiNetworkAdapter>,
}

impl DefaultIndexManager {
    /// Creates a new `DefaultIndexManager`.
    ///
    /// # Arguments
    ///
    /// * `storage_manager` - An `Arc` reference to a `DefaultStorageManager` implementation.
    /// * `network_adapter` - An `Arc` reference to a `AutonomiNetworkAdapter` implementation.
    ///
    /// # Returns
    ///
    /// A new `DefaultIndexManager` instance initialized with a default `MasterIndex`.
    pub fn new(
        storage_manager: Arc<DefaultStorageManager>,
        network_adapter: Arc<AutonomiNetworkAdapter>,
    ) -> Self {
        Self {
            state: Mutex::new(MasterIndex::default()),
            storage_manager,
            network_adapter,
        }
    }

    /// Loads the index from persistent storage or initializes a default one if loading fails.
    ///
    /// # Arguments
    ///
    /// * `master_index_address` - The network address where the master index is stored.
    /// * `master_index_key` - The secret key required to decrypt the master index.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` if a non-recoverable error occurs during loading (e.g., storage error).
    /// Logs warnings for recoverable issues like deserialization errors or missing index (starts fresh).
    pub async fn load_or_initialize(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Loading index from storage...");
        match load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            master_index_key,
        )
        .await
        {
            Ok(loaded_index) => {
                let mut state_guard = self.state.lock().await;
                *state_guard = loaded_index;
                info!("Index loaded successfully.");
                Ok(())
            }
            Err(e @ IndexError::DeserializationError(_)) | Err(e @ IndexError::Storage(_)) => {
                warn!(
                    "Index scratchpad not found or could not be loaded at {}: {}. Initializing default index.",
                    master_index_address, e
                );
                let mut state_guard = self.state.lock().await;
                *state_guard = MasterIndex::default();
                Ok(())
            }
            Err(e) => {
                error!("Critical error loading index: {}", e);
                Err(e)
            }
        }
    }

    /// Saves the current state of the index to persistent storage.
    ///
    /// # Arguments
    ///
    /// * `master_index_address` - The network address to save the index to.
    /// * `master_index_key` - The secret key used to encrypt the index before saving.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` if saving fails (e.g., serialization or storage error).
    pub async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        debug!("IndexManager: Saving index to storage...");
        let state_guard = self.state.lock().await;
        save_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            master_index_key,
            &state_guard,
        )
        .await?;
        info!("Index saved successfully to {}", master_index_address);
        Ok(())
    }

    /// Retrieves the `KeyInfo` associated with a given user key.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key to look up.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyInfo))` if the key exists, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.index.get(key).cloned())
    }

    /// Inserts or updates the `KeyInfo` for a given user key.
    ///
    /// If the key already exists, its `KeyInfo` is replaced.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key.
    /// * `info` - The `KeyInfo` to associate with the key.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        state_guard.index.insert(key, info);
        Ok(())
    }

    /// Removes the `KeyInfo` associated with a given user key.
    ///
    /// Also handles moving the pads previously associated with the key:
    /// - Pads with status Allocated, Written, or Confirmed are added to the free list.
    /// - Pads with status Generated are added to the pending verification list.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key to remove.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyInfo))` containing the removed info if the key existed, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let mut state_guard = self.state.lock().await;
        if let Some(removed_info) = state_guard.index.remove(key) {
            debug!(
                "Removed key '{}'. Processing its {} pads...",
                key,
                removed_info.pads.len()
            );
            let mut pads_to_free = Vec::new();
            let mut pads_to_verify = Vec::new();

            for pad in &removed_info.pads {
                if let Some(key_bytes) = removed_info.pad_keys.get(&pad.address) {
                    match pad.status {
                        PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                            debug!(
                                "Pad {} (status {:?}) from removed key '{}' marked for free list.",
                                pad.address, pad.status, key
                            );
                            pads_to_free.push((pad.address, key_bytes.clone()));
                        }
                        PadStatus::Generated => {
                            if pad.needs_reverification {
                                debug!(
                                     "Pad {} (status {:?}, needs_reverification=true) from removed key '{}' marked for verification queue.",
                                     pad.address, pad.status, key
                                 );
                                pads_to_verify.push((pad.address, key_bytes.clone()));
                            } else {
                                debug!(
                                    "Pad {} (status {:?}, needs_reverification=false) from removed key '{}' discarded.",
                                    pad.address, pad.status, key
                                );
                            }
                        }
                    }
                } else {
                    warn!(
                        "Secret key not found in KeyInfo for pad {} associated with removed key '{}'. Cannot process this pad.",
                        pad.address, key
                    );
                }
            }
            drop(state_guard);

            if !pads_to_free.is_empty() {
                debug!(
                    "Adding {} pads from removed key '{}' to the free list.",
                    pads_to_free.len(),
                    key
                );
                self.add_free_pads(pads_to_free).await?;
            }
            if !pads_to_verify.is_empty() {
                debug!(
                    "Adding {} pads from removed key '{}' to the verification queue.",
                    pads_to_verify.len(),
                    key
                );
                self.add_pending_pads(pads_to_verify).await?;
            }

            Ok(Some(removed_info))
        } else {
            Ok(None)
        }
    }

    /// Lists all user keys currently present in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` of user keys.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn list_keys(&self) -> Result<Vec<String>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.index.keys().cloned().collect())
    }

    /// Retrieves detailed metadata (`KeyDetails`) for a specific user key.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyDetails))` if the key exists, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    #[allow(dead_code)]
    pub async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.index.get(key).map(|info| {
            let percentage = if !info.is_complete && !info.pads.is_empty() {
                let confirmed_count = info
                    .pads
                    .iter()
                    .filter(|p| p.status == PadStatus::Confirmed)
                    .count();
                Some((confirmed_count as f32 / info.pads.len() as f32) * 100.0)
            } else {
                None
            };
            KeyDetails {
                key: key.to_string(),
                size: info.data_size,
                modified: info.modified,
                is_finished: info.is_complete,
                completion_percentage: percentage,
            }
        }))
    }

    /// Lists detailed metadata (`KeyDetails`) for all keys in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<KeyDetails>`.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard
            .index
            .iter()
            .map(|(key, info)| {
                let percentage = if !info.is_complete && !info.pads.is_empty() {
                    let confirmed_count = info
                        .pads
                        .iter()
                        .filter(|p| p.status == PadStatus::Confirmed)
                        .count();
                    Some((confirmed_count as f32 / info.pads.len() as f32) * 100.0)
                } else {
                    None
                };
                KeyDetails {
                    key: key.clone(),
                    size: info.data_size,
                    modified: info.modified,
                    is_finished: info.is_complete,
                    completion_percentage: percentage,
                }
            })
            .collect())
    }

    /// Calculates and returns storage statistics based on the current index state.
    ///
    /// # Returns
    ///
    /// A `StorageStats` struct.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn get_storage_stats(&self) -> Result<StorageStats, IndexError> {
        let state_guard = self.state.lock().await;
        let scratchpad_size = if state_guard.scratchpad_size == 0 {
            warn!("Index scratchpad size is 0, using default for stats calculation.");
            DEFAULT_SCRATCHPAD_SIZE
        } else {
            state_guard.scratchpad_size
        };

        let free_pads_count = state_guard.free_pads.len();
        let pending_verification_pads_count = state_guard.pending_verification_pads.len();

        let mut occupied_pads_count = 0;
        let mut occupied_data_size_total: u64 = 0;
        let mut allocated_written_pads_count = 0; // Pads backing incomplete keys but not yet Confirmed

        let mut incomplete_keys_count = 0;
        let mut incomplete_keys_data_bytes = 0;
        let mut incomplete_keys_total_pads = 0;
        let mut incomplete_keys_pads_generated = 0;
        let mut incomplete_keys_pads_written = 0;
        let mut incomplete_keys_pads_confirmed = 0; // Confirmed pads belonging to incomplete keys

        for key_info in state_guard.index.values() {
            if key_info.is_complete {
                // Fully confirmed keys contribute to occupied pads and data size
                occupied_pads_count += key_info.pads.len();
                occupied_data_size_total += key_info.data_size as u64;
            } else {
                // Incomplete keys analysis
                incomplete_keys_count += 1;
                incomplete_keys_data_bytes += key_info.data_size as u64;
                incomplete_keys_total_pads += key_info.pads.len();

                for pad_info in &key_info.pads {
                    match pad_info.status {
                        PadStatus::Generated => incomplete_keys_pads_generated += 1,
                        PadStatus::Allocated => allocated_written_pads_count += 1,
                        PadStatus::Written => {
                            incomplete_keys_pads_written += 1;
                            allocated_written_pads_count += 1;
                        }
                        PadStatus::Confirmed => {
                            incomplete_keys_pads_confirmed += 1;
                            // Confirmed pads of incomplete keys *also* count towards occupied
                            occupied_pads_count += 1;
                        }
                    }
                }
                // Include data size even for incomplete keys in the total occupied data calculation
                occupied_data_size_total += key_info.data_size as u64;
            }
        }

        let total_pads_managed = occupied_pads_count
            + allocated_written_pads_count // Pads for incomplete keys (non-confirmed)
            + free_pads_count
            + pending_verification_pads_count;

        let scratchpad_size_u64 = scratchpad_size as u64;

        // Space calculation
        let occupied_pad_space_bytes = occupied_pads_count as u64 * scratchpad_size_u64;
        let free_pad_space_bytes = free_pads_count as u64 * scratchpad_size_u64;
        let total_space_bytes = total_pads_managed as u64 * scratchpad_size_u64;

        let wasted_space_bytes = occupied_pad_space_bytes.saturating_sub(occupied_data_size_total);

        Ok(StorageStats {
            scratchpad_size,
            total_pads: total_pads_managed,
            occupied_pads: occupied_pads_count,
            free_pads: free_pads_count,
            pending_verification_pads: pending_verification_pads_count,
            total_space_bytes,
            occupied_pad_space_bytes,
            free_pad_space_bytes,
            occupied_data_bytes: occupied_data_size_total,
            wasted_space_bytes,
            incomplete_keys_count,
            incomplete_keys_data_bytes,
            incomplete_keys_total_pads,
            incomplete_keys_pads_generated,
            incomplete_keys_pads_written,
            incomplete_keys_pads_confirmed,
        })
    }

    /// Adds a single pad (address and key) to the free pad list.
    ///
    /// This method attempts to fetch the pad's current counter from storage before adding it.
    /// If fetching fails (e.g., network error), the pad is added with counter 0 and a warning logged.
    /// If the pad is not found via `check_existence`, it's not added and a warning is logged.
    ///
    /// # Arguments
    ///
    /// * `address` - The address of the pad.
    /// * `key_bytes` - The secret key bytes of the pad.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures related to index modification.
    pub async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError> {
        self.add_free_pads(vec![(address, key_bytes)]).await
    }

    /// Adds multiple verified pads (with keys) directly to the free list.
    ///
    /// This bypasses any network verification and assumes the caller guarantees the pads exist and are usable.
    /// Primarily used internally, e.g., after a successful `purge` operation.
    ///
    /// # Arguments
    ///
    /// * `pads` - A vector of tuples, each containing a `ScratchpadAddress` and its corresponding `SecretKey` bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures (e.g., lock contention).
    #[allow(dead_code)] // Suppress warning as this might be used externally or later
    pub async fn add_verified_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Pads here are assumed verified
    ) -> Result<(), IndexError> {
        if pads.is_empty() {
            return Ok(());
        }
        debug!("Adding {} verified pads to the free list.", pads.len());
        let mut state_guard = self.state.lock().await;
        for (address, key_bytes) in pads {
            // Verified pads always start with counter 0 in the free list
            state_guard.free_pads.push((address, key_bytes, 0));
        }
        Ok(())
    }

    /// Takes (removes and returns) one pad from the free list, if available.
    ///
    /// # Returns
    ///
    /// `Ok(Some((address, key_bytes, counter)))` if a pad was available, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn take_free_pad(
        &self,
    ) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(state_guard.free_pads.pop())
    }

    /// Takes (removes and returns) all pads currently in the pending verification list.
    ///
    /// # Returns
    ///
    /// A `Vec` containing tuples of (address, key_bytes) for the removed pending pads.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        let pending = std::mem::take(&mut state_guard.pending_verification_pads);
        Ok(pending)
    }

    /// Removes a specific pad address from the pending verification list.
    ///
    /// Used typically after a pending pad has been successfully verified and added to the free list.
    ///
    /// # Arguments
    ///
    /// * `address_to_remove` - The address of the pad to remove from the pending list.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    #[allow(dead_code)]
    pub async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        state_guard
            .pending_verification_pads
            .retain(|(addr, _)| addr != address_to_remove);
        Ok(())
    }

    /// Updates the status (`PadStatus`) of a specific pad associated with a given key.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key the pad belongs to.
    /// * `pad_address` - The address of the pad to update.
    /// * `new_status` - The new `PadStatus` to set.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::KeyNotFound` if the key doesn't exist.
    /// Returns `IndexError::PadNotFound` if the pad address isn't associated with the key.
    /// Returns other `IndexError` variants on internal failures.
    pub async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        let key_info = state_guard
            .index
            .get_mut(key)
            .ok_or(IndexError::KeyNotFound(key.to_string()))?;
        let pad_info = key_info
            .pads
            .iter_mut()
            .find(|p| p.address == *pad_address)
            .ok_or_else(|| {
                IndexError::InconsistentState(format!(
                    "Pad {} not found within key '{}'",
                    pad_address, key
                ))
            })?;
        pad_info.status = new_status;
        Ok(())
    }

    /// Marks a specific key as complete (`is_complete = true`) in its `KeyInfo`.
    ///
    /// This is typically called after all pads associated with a key reach the `Confirmed` status.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key to mark as complete.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::KeyNotFound` if the key doesn't exist.
    /// Returns other `IndexError` variants on internal failures.
    pub async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        let key_info = state_guard
            .index
            .get_mut(key)
            .ok_or(IndexError::KeyNotFound(key.to_string()))?;
        key_info.is_complete = true;
        Ok(())
    }

    /// Resets the index to its default state (empty keys, free pads, etc.) and saves it.
    ///
    /// # Arguments
    ///
    /// * `master_index_address` - The address to save the reset index to.
    /// * `master_index_key` - The key to encrypt the reset index with.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` if saving the reset index fails.
    pub async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Resetting index...");
        let mut state_guard = self.state.lock().await;
        *state_guard = MasterIndex::default();
        drop(state_guard);
        self.save(master_index_address, master_index_key).await?;
        info!("Index reset and saved successfully.");
        Ok(())
    }

    /// Returns a clone of the current in-memory `MasterIndex` state.
    ///
    /// # Returns
    ///
    /// A cloned `MasterIndex`.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures (e.g., mutex poisoning).
    pub async fn get_index_copy(&self) -> Result<MasterIndex, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.clone())
    }

    /// Directly replaces the current in-memory index with the provided `MasterIndex`.
    ///
    /// **Warning:** This bypasses normal loading/saving and should be used with caution,
    /// typically only after fetching and verifying a remote index.
    ///
    /// # Arguments
    ///
    /// * `new_index` - The `MasterIndex` to replace the current in-memory one.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures (e.g., mutex poisoning).
    pub async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError> {
        info!("IndexManager: Updating in-memory index directly.");
        let mut state_guard = self.state.lock().await;
        *state_guard = new_index;
        Ok(())
    }

    /// Retrieves the configured scratchpad size from the index.
    ///
    /// If the loaded index has a size of 0 (e.g., from an older version or corruption),
    /// it returns the `DEFAULT_SCRATCHPAD_SIZE`.
    ///
    /// # Returns
    ///
    /// The scratchpad size in bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn get_scratchpad_size(&self) -> Result<usize, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(if state_guard.scratchpad_size == 0 {
            warn!("Loaded index scratchpad size is 0, using default.");
            DEFAULT_SCRATCHPAD_SIZE
        } else {
            state_guard.scratchpad_size
        })
    }

    /// Attempts to fetch and deserialize the `MasterIndex` directly from its storage address.
    ///
    /// This bypasses the normal `load_or_initialize` logic and decryption, intended for
    /// scenarios like comparing a local index with the remote authoritative version.
    /// It uses a dummy key for the load function call, as decryption isn't the goal here.
    ///
    /// # Arguments
    ///
    /// * `master_index_address` - The address where the index is expected to be stored.
    ///
    /// # Returns
    ///
    /// The deserialized `MasterIndex` if successful.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` if fetching or deserialization fails.
    pub async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
    ) -> Result<MasterIndex, IndexError> {
        debug!("IndexManager: Fetching remote index directly...");
        let dummy_key = SecretKey::random();
        load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            &dummy_key,
        )
        .await
    }

    /// Adds multiple pads to the pending verification list.
    ///
    /// # Arguments
    ///
    /// * `pads` - A vector of tuples, each containing a pad's address and secret key bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        if pads.is_empty() {
            return Ok(());
        }
        let mut state_guard = self.state.lock().await;
        state_guard.pending_verification_pads.extend(pads);
        Ok(())
    }

    /// Adds multiple pads to the free pad list.
    ///
    /// Like `add_free_pad`, this attempts to fetch the counter for each pad before adding.
    /// Pads that fail the counter fetch are added with counter 0.
    /// Pads confirmed not to exist are skipped.
    ///
    /// # Arguments
    ///
    /// * `pads` - A vector of tuples, each containing a pad's address and secret key bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures related to index modification.
    pub async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        if pads.is_empty() {
            return Ok(());
        }
        debug!(
            "Attempting to add {} pads to free list (will check existence and fetch counter).",
            pads.len()
        );
        let mut pads_to_add_final = Vec::with_capacity(pads.len());

        for (address, key_bytes) in pads {
            trace!("Processing pad {} for free list addition.", address);
            match self.network_adapter.check_existence(&address).await {
                Ok(true) => {
                    trace!(
                        "Pad {} exists. Fetching scratchpad to get counter.",
                        address
                    );
                    match self.network_adapter.get_raw_scratchpad(&address).await {
                        Ok(scratchpad) => {
                            let counter = scratchpad.counter();
                            trace!(
                                "Pad {} fetched, counter is {}. Adding to final list.",
                                address,
                                counter
                            );
                            pads_to_add_final.push((address, key_bytes, counter));
                        }
                        Err(e) => {
                            warn!("Failed to fetch scratchpad for existing pad {}: {}. Adding to free list with counter 0.", address, e);
                            pads_to_add_final.push((address, key_bytes, 0));
                        }
                    }
                }
                Ok(false) => {
                    warn!(
                        "Pad {} reported as non-existent during free pad add check. Skipping.",
                        address
                    );
                }
                Err(e) => {
                    warn!("Network error checking existence for pad {}: {}. Adding to free list with counter 0 as fallback.", address, e);
                    pads_to_add_final.push((address, key_bytes, 0));
                }
            }
        }

        if !pads_to_add_final.is_empty() {
            debug!("Adding {} pads to free list.", pads_to_add_final.len());
            let mut state_guard = self.state.lock().await;
            state_guard.free_pads.extend(pads_to_add_final);
        } else {
            debug!("No pads were ultimately added to the free list after checks.");
        }
        Ok(())
    }
}
