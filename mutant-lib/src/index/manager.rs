use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::structure::{
    IndexEntry, KeyInfo, MasterIndex, PadStatus, PublicUploadInfo, DEFAULT_SCRATCHPAD_SIZE,
};
use crate::network::AutonomiNetworkAdapter;
use crate::types::{KeyDetails, KeySummary, StorageStats};
use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Default implementation of the `IndexManager` trait.
/// Manages the state of the index, including key info, free pads, and pending verification pads.
pub struct DefaultIndexManager {
    state: Mutex<MasterIndex>,
    network_adapter: Arc<AutonomiNetworkAdapter>,
    master_index_key: SecretKey,
}

impl DefaultIndexManager {
    /// Creates a new `DefaultIndexManager`.
    ///
    /// # Arguments
    ///
    /// * `network_adapter` - An `Arc` reference to a `AutonomiNetworkAdapter` implementation.
    /// * `master_index_key` - The secret key required to decrypt the master index.
    ///
    /// # Returns
    ///
    /// A new `DefaultIndexManager` instance initialized with a default `MasterIndex`.
    pub fn new(network_adapter: Arc<AutonomiNetworkAdapter>, master_index_key: SecretKey) -> Self {
        Self {
            state: Mutex::new(MasterIndex::default()),
            network_adapter,
            master_index_key,
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
            Err(e @ IndexError::DeserializationError(_)) => {
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
            self.network_adapter.as_ref(),
            master_index_address,
            master_index_key,
            &state_guard,
        )
        .await?;
        info!("Index saved successfully to {}", master_index_address);
        Ok(())
    }

    /// Retrieves a clone of the `KeyInfo` for a specific private key.
    /// Returns `Ok(None)` if the key doesn't exist or if it corresponds to a public upload.
    pub async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.index.get(key).and_then(|entry| match entry {
            IndexEntry::PrivateKey(info) => Some(info.clone()),
            IndexEntry::PublicUpload(_) => None, // Key exists, but is not a private key
        }))
    }

    /// Inserts or updates the `KeyInfo` for a specific private key.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key.
    /// * `info` - The `KeyInfo` to associate with the key.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::Lock` on internal lock failures.
    /// Returns `IndexError::KeyExists` if the key name already exists as a Public Upload.
    pub async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        // Check if the key exists as a public upload
        if let Some(IndexEntry::PublicUpload(_)) = state_guard.index.get(&key) {
            return Err(IndexError::KeyExists(key)); // Cannot overwrite public upload with private key
        }
        // Insert/overwrite as private key
        state_guard.index.insert(key, IndexEntry::PrivateKey(info)); // Wrap in enum variant
        Ok(())
    }

    /// Removes the entry (private key or public upload) for a specific key and returns the associated `KeyInfo` if it was a private key.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key or public upload name to remove.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyInfo))` if the key existed and was a private key.
    /// `Ok(None)` if the key did not exist or was a public upload.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::Lock` on internal lock failures.
    pub async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(state_guard.index.remove(key).and_then(|entry| match entry {
            IndexEntry::PrivateKey(info) => Some(info),
            IndexEntry::PublicUpload(_) => None, // Removed a public upload, return None as per doc
        }))
    }

    /// Lists all user keys and public upload names currently present in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<KeySummary>` containing summarized info for each entry.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn list_keys(&self) -> Result<Vec<KeySummary>, IndexError> {
        let state_guard = self.state.lock().await;
        let mut summaries = Vec::new();

        // Collect all entries from the unified index
        for (key, entry) in state_guard.index.iter() {
            match entry {
                IndexEntry::PrivateKey(_) => {
                    summaries.push(KeySummary {
                        name: key.clone(),
                        is_public: false,
                        address: None,
                    });
                }
                IndexEntry::PublicUpload(info) => {
                    summaries.push(KeySummary {
                        name: key.clone(),
                        is_public: true,
                        address: Some(info.address),
                    });
                }
            }
        }

        Ok(summaries)
    }

    /// Retrieves detailed metadata (`KeyDetails`) for a specific user key or public upload name.
    ///
    /// If the name exists as both a private key and a public upload, the private key details are prioritized.
    ///
    /// # Arguments
    ///
    /// * `key_or_name` - The user key or public upload name.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyDetails))` if the key/name exists, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    pub async fn get_key_details(
        &self,
        key_or_name: &str,
    ) -> Result<Option<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;

        match state_guard.index.get(key_or_name) {
            Some(IndexEntry::PrivateKey(info)) => {
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
                Ok(Some(KeyDetails {
                    key: key_or_name.to_string(),
                    size: info.data_size,
                    modified: info.modified,
                    is_finished: info.is_complete,
                    completion_percentage: percentage,
                    public_address: None, // This is a private key entry
                }))
            }
            Some(IndexEntry::PublicUpload(info)) => {
                // Construct KeyDetails for the public upload
                Ok(Some(KeyDetails {
                    key: key_or_name.to_string(),
                    size: info.size,
                    modified: info.modified,
                    is_finished: true, // Public uploads are inherently finished once created
                    completion_percentage: None,
                    public_address: Some(info.address),
                }))
            }
            None => Ok(None),
        }
    }

    /// Lists detailed metadata (`KeyDetails`) for all keys and public uploads in the index.
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
        let mut all_details = Vec::new();

        // Process all entries from the unified index
        for (key, entry) in state_guard.index.iter() {
            match entry {
                IndexEntry::PrivateKey(info) => {
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
                    all_details.push(KeyDetails {
                        key: key.clone(),
                        size: info.data_size,
                        modified: info.modified,
                        is_finished: info.is_complete,
                        completion_percentage: percentage,
                        public_address: None,
                    });
                }
                IndexEntry::PublicUpload(info) => {
                    all_details.push(KeyDetails {
                        key: key.clone(),
                        size: info.size,
                        modified: info.modified,
                        is_finished: true,
                        completion_percentage: None,
                        public_address: Some(info.address),
                    });
                }
            }
        }

        Ok(all_details)
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

        for entry in state_guard.index.values() {
            if let IndexEntry::PrivateKey(key_info) = entry {
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
            // Note: Public uploads currently do not contribute to these detailed pad stats.
            // We could add a count of public uploads or their total size if needed.
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

    /// Inserts metadata for a new public upload into the index.
    ///
    /// # Arguments
    ///
    /// * `name` - The unique name for the public upload.
    /// * `info` - The `PublicUploadInfo` containing metadata.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::KeyExists` if the name is already used (for a private key or public upload).
    /// Returns `IndexError::Lock` on internal lock failures.
    pub async fn insert_public_upload_info(
        &self,
        name: String,
        info: PublicUploadInfo,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        if state_guard.index.contains_key(&name) {
            return Err(IndexError::KeyExists(name));
        }
        state_guard
            .index
            .insert(name, IndexEntry::PublicUpload(info));
        Ok(())
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
        match state_guard.index.get_mut(key) {
            Some(IndexEntry::PrivateKey(key_info)) => {
                // Find the pad with the matching address
                if let Some(pad_info) = key_info
                    .pads // Access pads on key_info
                    .iter_mut()
                    .find(|p| p.address == *pad_address)
                {
                    debug!(
                        "IndexManager: Updating pad {} for key '{}' to status {:?}",
                        pad_address, key, new_status
                    );
                    pad_info.status = new_status;
                    pad_info.needs_reverification = false; // Assume status update implies verification not needed
                    key_info.modified = Utc::now();
                    Ok(())
                } else {
                    warn!(
                        "IndexManager: Pad address {} not found for key '{}' during status update.",
                        pad_address, key
                    );
                    Err(IndexError::PadNotFound(*pad_address, key.to_string()))
                }
            }
            Some(IndexEntry::PublicUpload(_)) => {
                warn!(
                    "IndexManager: Attempted to update pad status for a public upload key '{}'",
                    key
                );
                Err(IndexError::KeyNotFound(key.to_string())) // Or a more specific error?
            }
            None => Err(IndexError::KeyNotFound(key.to_string())),
        }
    }

    /// Marks a specific private key as complete.
    ///
    /// Sets the `is_complete` flag to true for the `KeyInfo` associated with the key.
    /// Does nothing if the key does not exist or is a public upload.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::Lock` on internal lock failures.
    pub async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        if let Some(IndexEntry::PrivateKey(key_info)) = state_guard.index.get_mut(key) {
            key_info.is_complete = true; // Access is_complete on key_info
            key_info.modified = Utc::now();
            debug!("IndexManager: Marked key '{}' as complete.", key);
        } else {
            warn!(
                "IndexManager: Key '{}' not found or is public upload during mark_key_complete.",
                key
            );
            // Don't return error if not found or public, just do nothing as per doc.
        }
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
    /// It uses the actual master index key stored in the manager instead of a dummy key.
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
        load_index(
            self.network_adapter.as_ref(),
            master_index_address,
            &self.master_index_key,
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
            trace!("Fetching scratchpad for pad {} to get counter.", address);
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
                    // If fetching fails, it implies non-existence or network issue.
                    // Log appropriately and add with counter 0 as a fallback.
                    warn!(
                        "Failed to fetch scratchpad for pad {} (may not exist or network error): {}. Adding to free list with counter 0.",
                        address, e
                    );
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

    /// Processes pads from a removed KeyInfo, adding them to the appropriate
    /// free or pending verification lists locally without network checks.
    pub(crate) async fn harvest_pads(&self, key_info: KeyInfo) -> Result<(), IndexError> {
        debug!(
            "IndexManager: Harvesting {} pads locally.",
            key_info.pads.len()
        );
        // Collect pads to add locally to avoid holding lock during iteration
        let mut pads_to_add_free_locally: Vec<(ScratchpadAddress, Vec<u8>, u64)> = Vec::new();
        let mut pads_to_add_pending_locally: Vec<(ScratchpadAddress, Vec<u8>)> = Vec::new();

        for pad_info in key_info.pads {
            if let Some(key_bytes) = key_info.pad_keys.get(&pad_info.address) {
                match pad_info.status {
                    PadStatus::Generated => {
                        trace!(
                            "Harvesting pad {} (Generated) to pending verification list (local)",
                            pad_info.address
                        );
                        pads_to_add_pending_locally.push((pad_info.address, key_bytes.clone()));
                    }
                    PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                        trace!(
                            "Harvesting pad {} ({:?}) to free list with counter 0 (local)",
                            pad_info.address,
                            pad_info.status
                        );
                        // Add directly with counter 0, network check/fetch happens on reuse/purge
                        pads_to_add_free_locally.push((pad_info.address, key_bytes.clone(), 0));
                    }
                }
            } else {
                warn!(
                    "Could not find key for pad {} during harvesting. Skipping.",
                    pad_info.address
                );
            }
        }

        // Acquire lock once and update both lists
        if !pads_to_add_free_locally.is_empty() || !pads_to_add_pending_locally.is_empty() {
            debug!(
                "Adding {} pads to free list and {} pads to pending list (locally).",
                pads_to_add_free_locally.len(),
                pads_to_add_pending_locally.len()
            );
            let mut state_guard = self.state.lock().await;
            state_guard.free_pads.extend(pads_to_add_free_locally);
            state_guard
                .pending_verification_pads
                .extend(pads_to_add_pending_locally);
        } else {
            debug!("No pads harvested.");
        }

        debug!("Local pad harvesting complete.");
        Ok(())
    }
}
