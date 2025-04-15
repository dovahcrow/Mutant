use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::query;
use crate::index::structure::{KeyInfo, MasterIndex, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::NetworkAdapter;
use crate::storage::StorageManager;
use crate::types::{KeyDetails, StorageStats};
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::error;
use log::{debug, info, warn};
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
        master_index_key: &SecretKey,
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

    /// Takes a single pad from the free list, if available.
    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>)>, IndexError>;

    /// Adds multiple pads to the pending verification list.
    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;

    /// Takes all pads currently in the pending verification list.
    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError>;

    /// Updates the free/pending lists based on verification results.
    async fn update_pad_lists(
        &self,
        verified: Vec<(ScratchpadAddress, Vec<u8>)>,
        failed_count: usize,
    ) -> Result<(), IndexError>;

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
    ) -> Result<MasterIndex, IndexError>;
}

// --- Implementation ---

pub struct DefaultIndexManager {
    state: Arc<Mutex<MasterIndex>>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultIndexManager {
    pub fn new(
        storage_manager: Arc<dyn StorageManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(MasterIndex::default())), // Start with default
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
        master_index_key: &SecretKey, // Key needed if we need to create/save default
    ) -> Result<(), IndexError> {
        info!("IndexManager: Loading index from storage...");
        match load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
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
            Err(IndexError::KeyNotFound(_)) => {
                info!("Index not found in storage. Initializing with default in memory.");
                // State already holds default, so nothing to do here for the in-memory part.
                // We don't automatically save the default here; that's handled by the API layer
                // based on user interaction (prompt).
                Ok(())
            }
            Err(e) => {
                error!("Failed to load index from storage: {}", e);
                Err(e)
            }
        }
    }

    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        debug!("IndexManager: Saving index to storage...");
        let state_guard = self.state.lock().await;
        save_index(
            self.storage_manager.as_ref(),
            master_index_address,
            master_index_key,
            &*state_guard, // Pass the locked state
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
        Ok(query::remove_key_info_internal(&mut *state_guard, key))
        // Note: Does not automatically save. Higher layers decide when to persist.
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
        let mut state_guard = self.state.lock().await;
        query::add_free_pad_internal(&mut *state_guard, address, key_bytes)
        // Note: Does not automatically save.
    }

    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_free_pad_internal(&mut *state_guard))
        // Note: Does not automatically save.
    }

    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::add_pending_pads_internal(&mut *state_guard, pads)
        // Note: Does not automatically save.
    }

    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_pending_pads_internal(&mut *state_guard))
        // Note: Does not automatically save.
    }

    async fn update_pad_lists(
        &self,
        verified: Vec<(ScratchpadAddress, Vec<u8>)>,
        failed_count: usize,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::update_pad_lists_internal(&mut *state_guard, verified, failed_count)
        // Note: Does not automatically save. Purge logic saves cache separately.
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
    ) -> Result<MasterIndex, IndexError> {
        debug!("IndexManager: Fetching remote index directly...");
        load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
        )
        .await
    }
}
