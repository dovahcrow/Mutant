use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::query;
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::StorageManager;
use crate::types::{KeyDetails, StorageStats};
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Trait defining the interface for managing the Master Index.
/// Handles loading, saving, querying, and modifying the index state.
#[async_trait]
pub trait IndexManager: Send + Sync {
    /// Loads the index from persistence (using StorageManager) or initializes a default one in memory.
    /// This should be called once during initialization.
    async fn load_or_initialize(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    /// Saves the current in-memory index state to persistence.
    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    /// Retrieves a clone of the KeyInfo for a specific key.
    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

    /// Inserts or updates the KeyInfo for a specific key.
    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError>;

    /// Removes the KeyInfo for a specific key, returning the old info if it existed.
    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

    /// Lists all user keys currently stored.
    async fn list_keys(&self) -> Result<Vec<String>, IndexError>;

    /// Retrieves detailed information (KeyDetails) for a specific key.
    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError>;

    /// Retrieves detailed information (KeyDetails) for all keys.
    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError>;

    /// Calculates and returns current storage statistics.
    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError>;

    /// Adds a pad (address and key bytes) to the free list.
    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError>;

    /// Adds multiple pads (address and key bytes) to the free list.
    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;

    /// Takes a single pad from the free list, if available.
    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError>;

    /// Takes all pads currently in the pending verification list.
    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError>;

    /// Removes a specific pad (address) from the pending verification list.
    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError>;

    /// Updates the status of a specific pad within a key's metadata.
    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError>;

    /// Marks a key as fully complete (all pads confirmed).
    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError>;

    /// Resets the in-memory index to default and persists the reset.
    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    /// Returns a clone of the current in-memory MasterIndex. Use with caution for sync/backup.
    async fn get_index_copy(&self) -> Result<MasterIndex, IndexError>;

    /// Overwrites the current in-memory MasterIndex with the provided one. Use with caution for sync/restore.
    async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError>;

    /// Gets the configured scratchpad size from the index.
    async fn get_scratchpad_size(&self) -> Result<usize, IndexError>;

    /// Fetches the MasterIndex directly from the persistence layer (remote), bypassing in-memory state.
    async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
        // NOTE: This function CANNOT decrypt the loaded index without the key!
        // It will likely return a DecryptionError if the index exists.
    ) -> Result<MasterIndex, IndexError>;

    /// Adds multiple pads (address and key bytes) to the pending verification list.
    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Use Vec<u8> for key
    ) -> Result<(), IndexError>;
}

// --- Implementation ---

pub struct DefaultIndexManager {
    state: Mutex<MasterIndex>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultIndexManager {
    pub fn new(
        storage_manager: Arc<dyn StorageManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            state: Mutex::new(MasterIndex::default()),
            storage_manager,
            network_adapter,
        }
    }
}

#[async_trait]
impl IndexManager for DefaultIndexManager {
    async fn load_or_initialize(
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
                // Ensure loaded index has a valid scratchpad size
                if loaded_index.scratchpad_size == 0 {
                    warn!(
                        "Loaded index has scratchpad_size 0, setting to default: {}",
                        DEFAULT_SCRATCHPAD_SIZE
                    );
                    *state_guard = MasterIndex {
                        scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                        ..loaded_index
                    };
                    // Persist the corrected index? Or wait for next explicit save?
                    // Let's wait for explicit save for now to avoid unnecessary writes during init.
                } else {
                    *state_guard = loaded_index;
                }
                info!("Index loaded successfully from storage.");
                Ok(())
            }
            // Treat ANY deserialization error (not found, corrupted, version mismatch) as a reason to initialize default.
            Err(IndexError::DeserializationError(e)) => {
                warn!(
                    "Failed to load or deserialize index from storage ({}). Initializing with default in memory.",
                    e
                );
                // State already holds default, so nothing to change in memory.
                // We don't automatically save the default here; PadLifecycleManager handles caching if needed.
                Ok(())
            }
            // Re-introduce handling for KeyNotFound just in case the persistence layer changes behavior.
            // This path should ideally not be hit if persistence correctly returns DeserializationError for not found.
            Err(IndexError::KeyNotFound(_)) => {
                warn!("Index key not found (unexpected persistence state?). Initializing with default in memory.");
                Ok(())
            }
            // Propagate other errors (e.g., storage connection issues)
            Err(e) => {
                error!("Failed to load index from storage: {}", e);
                Err(e)
            }
        }
    }

    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        debug!("IndexManager: Saving index to storage...");
        let state_guard = self.state.lock().await;
        save_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            _master_index_key,
            &*state_guard,
        )
        .await
    }

    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_info_internal(&*state_guard, key).cloned())
    }

    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::insert_key_info_internal(&mut *state_guard, key, info)
        // Note: Does not automatically save. Higher layers decide when to persist.
    }

    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let mut state_guard = self.state.lock().await;
        let removed_info = query::remove_key_info_internal(&mut *state_guard, key);

        if let Some(ref info) = removed_info {
            let mut pads_to_free = Vec::new();
            let mut pads_to_verify = Vec::new();

            for pad_info in &info.pads {
                if let Some(pad_key) = info.pad_keys.get(&pad_info.address).cloned() {
                    match pad_info.status {
                        PadStatus::Confirmed => {
                            trace!(
                                "Pad {} for removed key '{}' is Confirmed. Releasing to free pool.",
                                pad_info.address,
                                key
                            );
                            // Determine counter based on origin
                            let counter = match pad_info.origin {
                                PadOrigin::FreePool { initial_counter } => initial_counter,
                                PadOrigin::Generated => 0, // Assume 0 if generated and somehow confirmed?
                            };
                            pads_to_free.push((pad_info.address, pad_key, counter));
                        }
                        PadStatus::Generated | PadStatus::Allocated | PadStatus::Written => {
                            trace!(
                                "Pad {} for removed key '{}' is {:?}. Adding to pending verification.",
                                pad_info.address, key, pad_info.status
                            );
                            // Pending verification only needs address and key
                            pads_to_verify.push((pad_info.address, pad_key));
                        }
                    }
                } else {
                    warn!(
                        "Key '{}', Pad {}: Missing key bytes in KeyInfo during removal. Cannot process.",
                        key,
                        pad_info.address
                    );
                }
            }

            if !pads_to_free.is_empty() {
                debug!(
                    "Key '{}': Adding {} pads with counters to the free list.",
                    key,
                    pads_to_free.len()
                );
                // Use the modified internal function that accepts counters
                query::add_free_pads_with_counters_internal(&mut *state_guard, pads_to_free)?;
            }
            if !pads_to_verify.is_empty() {
                debug!(
                    "Key '{}': Adding {} pads to the pending verification list.",
                    key,
                    pads_to_verify.len()
                );
                query::add_pending_verification_pads_internal(&mut *state_guard, pads_to_verify)?;
            }
        }

        Ok(removed_info)
    }

    async fn list_keys(&self) -> Result<Vec<String>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_keys_internal(&*state_guard))
    }

    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_details_internal(&*state_guard, key))
    }

    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_all_key_details_internal(&*state_guard))
    }

    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError> {
        let state_guard = self.state.lock().await;
        query::get_stats_internal(&*state_guard)
    }

    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError> {
        // Fetch counter before adding
        match self.storage_manager.read_pad_scratchpad(&address).await {
            Ok(scratchpad) => {
                let counter = scratchpad.counter();
                trace!(
                    "Fetched counter {} for pad {} before adding to free list.",
                    counter,
                    address
                );
                let mut state_guard = self.state.lock().await;
                query::add_free_pad_with_counter_internal(
                    &mut *state_guard,
                    address,
                    key_bytes,
                    counter,
                )
            }
            Err(e) => {
                warn!("Failed to fetch scratchpad to get counter for pad {} during add_free_pad: {}. Pad will not be added to free list.", address, e);
                // Return Ok even if fetch failed, as the primary goal (not adding a bad entry) is met?
                // Or return an error? Let's return Ok, but log the warning.
                Ok(())
            }
        }
    }

    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Takes pads without counters
    ) -> Result<(), IndexError> {
        let mut pads_with_counters = Vec::with_capacity(pads.len());
        for (address, key_bytes) in pads {
            match self.storage_manager.read_pad_scratchpad(&address).await {
                Ok(scratchpad) => {
                    let counter = scratchpad.counter();
                    trace!(
                        "Fetched counter {} for pad {} before adding to free list.",
                        counter,
                        address
                    );
                    pads_with_counters.push((address, key_bytes, counter));
                }
                Err(e) => {
                    warn!("Failed to fetch scratchpad to get counter for pad {} during add_free_pads: {}. Pad will not be added to free list.", address, e);
                    // Skip adding this pad
                }
            }
        }

        if !pads_with_counters.is_empty() {
            let mut state_guard = self.state.lock().await;
            // Use the modified internal function
            query::add_free_pads_with_counters_internal(&mut *state_guard, pads_with_counters)?;
        }
        Ok(())
    }

    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_free_pad_internal(&mut *state_guard))
    }

    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_pending_pads_internal(&mut *state_guard))
        // Note: Does not automatically save.
    }

    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::remove_from_pending_internal(&mut *state_guard, address_to_remove)
        // Note: Does not automatically save.
    }

    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::update_pad_status_internal(&mut *state_guard, key, pad_address, new_status)
        // Note: Does not automatically save.
    }

    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::mark_key_complete_internal(&mut *state_guard, key)
        // Note: Does not automatically save.
    }

    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Resetting index...");
        {
            // Scope for the lock guard
            let mut state_guard = self.state.lock().await;
            query::reset_index_internal(&mut *state_guard);
        } // Lock released here
          // Persist the reset state immediately
        self.save(master_index_address, master_index_key).await?;
        info!("Index reset and saved successfully.");
        Ok(())
    }

    async fn get_index_copy(&self) -> Result<MasterIndex, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.clone())
    }

    async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError> {
        info!("IndexManager: Updating in-memory index directly.");
        let mut state_guard = self.state.lock().await;
        *state_guard = new_index;
        Ok(())
        // Note: Does not automatically save. Sync logic handles persistence.
    }

    async fn get_scratchpad_size(&self) -> Result<usize, IndexError> {
        let state_guard = self.state.lock().await;
        let size = state_guard.scratchpad_size;
        if size == 0 {
            warn!("Index reports scratchpad size of 0, returning default.");
            Ok(DEFAULT_SCRATCHPAD_SIZE)
            // Or return error? Err(IndexError::InconsistentState("Scratchpad size is zero".to_string()))
        } else {
            Ok(size)
        }
    }
    // Removed extra brace here

    async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
        // NOTE: This function CANNOT decrypt the loaded index without the key!
        // It will likely return a DecryptionError if the index exists.
    ) -> Result<MasterIndex, IndexError> {
        debug!("IndexManager: Fetching remote index directly...");
        // HACK: Create a dummy key because we don't have the real one here.
        // load_index will attempt decryption and fail if the index exists.
        let dummy_key = SecretKey::random();
        load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            &dummy_key, // Pass dummy key
        )
        .await
    }

    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Use Vec<u8> for key
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::add_pending_pads_internal(&mut *state_guard, pads)
        // Note: Does not automatically save.
    }
}
