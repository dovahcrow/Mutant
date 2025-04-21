use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::structure::{
    IndexEntry, KeyInfo, MasterIndex, PadStatus, PublicUploadInfo, DEFAULT_SCRATCHPAD_SIZE,
};
use crate::network::AutonomiNetworkAdapter;
use crate::types::{KeyDetails, KeySummary, StorageStats};
use autonomi::client::payment::Receipt;
use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use log::{debug, error, info, trace, warn};
use std::collections::HashSet;
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
            let (is_public, address) = match entry {
                IndexEntry::PrivateKey(_) => (false, None),
                IndexEntry::PublicUpload(info) => (true, Some(info.index_pad.address)),
            };
            summaries.push(KeySummary {
                name: key.clone(),
                is_public,
                address,
            });
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
                    public_address: Some(info.index_pad.address),
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
            let details = match entry {
                IndexEntry::PrivateKey(info) => {
                    // Calculate completion percentage specifically for private keys
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
                        completion_percentage: percentage, // Use calculated percentage
                        public_address: None,
                    }
                }
                IndexEntry::PublicUpload(info) => {
                    // Public uploads don't have completion percentage in this sense
                    KeyDetails {
                        key: key.clone(),
                        size: info.size,
                        modified: info.modified,
                        is_finished: true,
                        completion_percentage: None,
                        public_address: Some(info.index_pad.address),
                    }
                }
            };
            all_details.push(details);
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

        // --- Private Key Stats Initialization ---
        let mut occupied_pads_count = 0;
        let mut occupied_data_size_total: u64 = 0;
        let mut allocated_written_pads_count = 0; // Renamed for clarity

        let mut incomplete_keys_count = 0;
        let mut incomplete_keys_data_bytes = 0;
        let mut incomplete_keys_total_pads = 0;
        let mut incomplete_keys_pads_generated = 0;
        let mut incomplete_keys_pads_written = 0;
        let mut incomplete_keys_pads_confirmed = 0; // Confirmed pads belonging to incomplete keys

        // --- Public Key Stats Initialization ---
        let mut public_index_count = 0;
        let mut public_data_actual_bytes: u64 = 0;
        let mut unique_public_pads = HashSet::new(); // Tracks unique index AND data pads
        let unique_public_data_only_pads: HashSet<ScratchpadAddress> = HashSet::new(); // Tracks unique data pads only (for wasted space calc)

        // --- Iterating through all index entries ---
        for entry in state_guard.index.values() {
            match entry {
                IndexEntry::PrivateKey(key_info) => {
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
                                PadStatus::Allocated => allocated_written_pads_count += 1, // Tracks pads allocated/written but not confirmed
                                PadStatus::Written => {
                                    incomplete_keys_pads_written += 1;
                                    allocated_written_pads_count += 1;
                                }
                                PadStatus::Confirmed => {
                                    incomplete_keys_pads_confirmed += 1;
                                    // Confirmed pads of incomplete keys *also* count towards occupied
                                    occupied_pads_count += 1;
                                    // Note: allocated_written_pads_count tracks non-confirmed pads only.
                                }
                            }
                        }
                        // Include data size even for incomplete keys in the total occupied data calculation
                        occupied_data_size_total += key_info.data_size as u64;
                    }
                }
                IndexEntry::PublicUpload(upload_info) => {
                    public_index_count += 1;
                    public_data_actual_bytes += upload_info.size as u64;

                    // Add index pad to the set of all public pads
                    unique_public_pads.insert(upload_info.index_pad.address);

                    // Check for orphaned public uploads
                    if upload_info.data_pads.is_empty() && upload_info.size > 0 {
                        warn!(
                            "Public upload '{}' has size {} but no data pads listed.",
                            upload_info.index_pad.address, upload_info.size
                        );
                    } else if upload_info.data_pads.is_empty() && upload_info.size == 0 {
                        // Size 0, no pads - this is fine for an empty upload
                        trace!(
                            "Public upload '{}' is empty (size 0, no pads).",
                            upload_info.index_pad.address
                        );
                    } else {
                        // Iterate through PadInfo in data_pads
                        for data_pad_info in &upload_info.data_pads {
                            unique_public_pads.insert(data_pad_info.address);
                        }
                    }
                }
            }
        }

        // --- Calculations ---
        let scratchpad_size_u64 = scratchpad_size as u64;

        // Calculate private key space
        let occupied_pad_space_bytes = occupied_pads_count as u64 * scratchpad_size_u64;
        let free_pad_space_bytes = free_pads_count as u64 * scratchpad_size_u64;
        let wasted_space_bytes = occupied_pad_space_bytes.saturating_sub(occupied_data_size_total);

        // Calculate public key space
        let public_index_space_bytes = public_index_count as u64 * scratchpad_size_u64; // Space for index pads
        let public_data_pad_count = unique_public_pads.len(); // Count of unique public index + data pads
        let public_data_space_bytes = public_data_pad_count as u64 * scratchpad_size_u64; // Space for unique index + data pads

        // Calculate wasted space based on *data* pads only
        let public_data_only_pad_count = unique_public_data_only_pads.len();
        let public_data_only_space_bytes = public_data_only_pad_count as u64 * scratchpad_size_u64;
        let public_data_wasted_bytes =
            public_data_only_space_bytes.saturating_sub(public_data_actual_bytes);

        // Calculate total pads (including public index + data pads)
        let total_pads_managed = occupied_pads_count // Confirmed pads (private, including incomplete confirmed)
            + allocated_written_pads_count // Allocated/Written pads for incomplete private keys (not confirmed)
            + free_pads_count
            + pending_verification_pads_count
            + public_data_pad_count; // Unique public index + data pads

        let total_space_bytes = total_pads_managed as u64 * scratchpad_size_u64;

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
            incomplete_keys_pads_allocated_written: allocated_written_pads_count,

            public_index_count,
            public_index_space_bytes, // Space for index pads specifically
            public_data_pad_count,    // Unique count of *all* public pads (index + data)
            public_data_space_bytes,  // Space for *all* unique public pads (index + data)
            public_data_actual_bytes,
            public_data_wasted_bytes, // Wasted = (Space of data pads only) - actual data size
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
        // Check if the key exists as a private key
        if let Some(IndexEntry::PrivateKey(_)) = state_guard.index.get(&name) {
            // It's generally okay to overwrite private with public IF NEEDED, but let's prevent it for now.
            // Revisit if use case arises. For now, explicit delete is safer.
            warn!(
                "Attempted to insert public upload info for '{}', but it already exists as a private key. Operation aborted.",
                name
            );
            return Err(IndexError::KeyExists(name)); // Prevent overwriting private with public
        }

        // Check if it already exists as a public upload (potential resume/overwrite scenario)
        if let Some(IndexEntry::PublicUpload(_)) = state_guard.index.get(&name) {
            warn!("Overwriting existing public upload info for '{}'", name);
            // Allow overwrite for public uploads
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
    /// Optionally updates the last known counter for the pad.
    ///
    /// # Arguments
    ///
    /// * `key` - The user key the pad belongs to.
    /// * `pad_address` - The address of the pad to update.
    /// * `new_status` - The new `PadStatus` to set.
    /// * `new_counter` - If `Some`, updates the `last_known_counter` field of the `PadInfo`.
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
        new_counter: Option<u64>,
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
                        "IndexManager: Updating pad {} for key '{}' to status {:?} (Counter: {:?})",
                        pad_address, key, new_status, new_counter
                    );
                    pad_info.status = new_status;
                    if let Some(counter) = new_counter {
                        pad_info.last_known_counter = counter;
                    }
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
            // Iterate directly over owned pads
            // The secret key is now part of PadInfo
            if !pad_info.sk_bytes.is_empty() {
                match pad_info.status {
                    PadStatus::Generated => {
                        trace!(
                            "Harvesting pad {} (Generated) to pending verification list (local)",
                            pad_info.address
                        );
                        pads_to_add_pending_locally.push((pad_info.address, pad_info.sk_bytes));
                    }
                    PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                        // Use the last known counter stored in PadInfo
                        let counter_to_add = pad_info.last_known_counter;
                        trace!(
                            "Harvesting pad {} ({:?}) to free list with last known counter {} (local)",
                            pad_info.address,
                            pad_info.status,
                            counter_to_add
                        );
                        pads_to_add_free_locally.push((
                            pad_info.address,
                            pad_info.sk_bytes, // Use sk_bytes from PadInfo
                            counter_to_add,
                        ));
                    }
                }
            } else {
                warn!(
                    "Secret key bytes missing for pad {} during harvesting. Skipping.",
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

    /// Returns the set of scratchpad addresses currently occupied by private key pads.
    pub async fn get_occupied_private_pad_addresses(
        &self,
    ) -> Result<HashSet<ScratchpadAddress>, IndexError> {
        let state_guard = self.state.lock().await;
        let occupied_pads = state_guard
            .index
            .values()
            .filter_map(|entry| match entry {
                IndexEntry::PrivateKey(key_info) => Some(key_info),
                IndexEntry::PublicUpload(_) => None,
            })
            .flat_map(|key_info| key_info.pads.iter().map(|pad_info| pad_info.address))
            .collect::<HashSet<ScratchpadAddress>>();
        Ok(occupied_pads)
    }

    /// Retrieves the full `PublicUploadInfo` for a specific public upload name.
    /// Returns `Ok(None)` if the name doesn't exist or if it corresponds to a private key.
    pub async fn get_public_upload_info(
        &self,
        name: &str,
    ) -> Result<Option<PublicUploadInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.index.get(name).and_then(|entry| match entry {
            IndexEntry::PublicUpload(info) => Some(info.clone()),
            IndexEntry::PrivateKey(_) => None, // Name exists, but is not a public upload
        }))
    }

    /// Replaces the entire `PublicUploadInfo` for a given name. Use with caution.
    /// Primarily intended for recovery or complex update scenarios.
    /// Consider using specific update methods (`update_public_data_pad_info`, `update_public_index_pad_info`) for standard updates.
    pub async fn replace_public_upload_info(
        &self,
        name: &str,
        new_info: PublicUploadInfo,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        match state_guard.index.get_mut(name) {
            Some(entry @ IndexEntry::PublicUpload(_)) => {
                *entry = IndexEntry::PublicUpload(new_info);
                Ok(())
            }
            Some(IndexEntry::PrivateKey(_)) => {
                error!(
                    "Attempted to replace public upload info for '{}', but it is a private key",
                    name
                );
                Err(IndexError::NotAPublicUpload(name.to_string()))
            }
            None => {
                error!("Public upload '{}' not found for replacement", name);
                Err(IndexError::PublicUploadNotFound(name.to_string()))
            }
        }
    }

    /// Updates the status and optionally the receipt of a specific data pad within a public upload entry.
    ///
    /// # Arguments
    ///
    /// * `name` - The name identifier of the public upload.
    /// * `chunk_index` - The index of the data chunk/pad to update.
    /// * `new_status` - The new status to set for the data pad.
    /// * `receipt` - An optional receipt to store for the data pad.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::PublicUploadNotFound` if no entry exists for `name`.
    /// Returns `IndexError::NotAPublicUpload` if the entry for `name` is a private key.
    /// Returns `IndexError::DataPadNotFound` if no data pad with `chunk_index` is found.
    pub async fn update_public_data_pad_info(
        &self,
        name: &str,
        chunk_index: usize,
        new_status: PadStatus,
        receipt: Option<Receipt>,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        match state_guard.index.get_mut(name) {
            Some(IndexEntry::PublicUpload(public_info)) => {
                if let Some(data_pad) = public_info
                    .data_pads
                    .iter_mut()
                    .find(|p| p.chunk_index == chunk_index)
                {
                    trace!(
                        "Updating data pad {} for public upload '{}' to status {:?}, receipt: {}",
                        chunk_index,
                        name,
                        new_status,
                        receipt.is_some()
                    );
                    data_pad.status = new_status;
                    if receipt.is_some() {
                        data_pad.receipt = receipt;
                    }
                    // Potentially update modified time?
                    // public_info.modified = Utc::now();
                    Ok(())
                } else {
                    error!(
                        "Data pad with index {} not found for public upload '{}'",
                        chunk_index, name
                    );
                    Err(IndexError::DataPadNotFound(name.to_string(), chunk_index))
                }
            }
            Some(IndexEntry::PrivateKey(_)) => {
                error!(
                    "Attempted to update public data pad for '{}', but it is a private key",
                    name
                );
                Err(IndexError::NotAPublicUpload(name.to_string()))
            }
            None => {
                error!("Public upload '{}' not found for data pad update", name);
                Err(IndexError::PublicUploadNotFound(name.to_string()))
            }
        }
    }

    /// Updates the status and optionally the receipt of the index pad within a public upload entry.
    ///
    /// # Arguments
    ///
    /// * `name` - The name identifier of the public upload.
    /// * `new_status` - The new status to set for the index pad.
    /// * `receipt` - An optional receipt to store for the index pad.
    ///
    /// # Errors
    ///
    /// Returns `IndexError::PublicUploadNotFound` if no entry exists for `name`.
    /// Returns `IndexError::NotAPublicUpload` if the entry for `name` is a private key.
    pub async fn update_public_index_pad_info(
        &self,
        name: &str,
        new_status: PadStatus,
        receipt: Option<Receipt>,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        match state_guard.index.get_mut(name) {
            Some(IndexEntry::PublicUpload(public_info)) => {
                trace!(
                    "Updating index pad for public upload '{}' to status {:?}, receipt: {}",
                    name,
                    new_status,
                    receipt.is_some()
                );
                public_info.index_pad.status = new_status;
                if receipt.is_some() {
                    public_info.index_pad.receipt = receipt;
                }
                // Update modified time when the index pad (the main entry point) is confirmed
                public_info.modified = Utc::now();
                Ok(())
            }
            Some(IndexEntry::PrivateKey(_)) => {
                error!(
                    "Attempted to update public index pad for '{}', but it is a private key",
                    name
                );
                Err(IndexError::NotAPublicUpload(name.to_string()))
            }
            None => {
                error!("Public upload '{}' not found for index pad update", name);
                Err(IndexError::PublicUploadNotFound(name.to_string()))
            }
        }
    }
}
