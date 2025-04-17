use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::query;
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::NetworkAdapter;
use crate::storage::StorageManager;
use crate::types::{KeyDetails, StorageStats};
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Trait defining the interface for managing the MutAnt index.
///
/// Implementations are responsible for maintaining the state of the index, including:
/// - Key information (`KeyInfo`) mapping user keys to data locations and metadata.
/// - Free pad list: Available storage pads ready for use.
/// - Pending verification list: Pads that need checking before being added to the free list.
///
/// It handles loading/saving the index, querying its state, and modifying it based on
/// data operations (store, remove) and pad lifecycle events.
#[async_trait]
pub trait IndexManager: Send + Sync {
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
    async fn load_or_initialize(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

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
    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

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
    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

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
    /// Returns `IndexError::KeyAlreadyExists` if the key already exists (though the default impl overwrites).
    /// Returns other `IndexError` variants on internal failures.
    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError>;

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
    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

    /// Lists all user keys currently present in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` of user keys.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn list_keys(&self) -> Result<Vec<String>, IndexError>;

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
    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError>;

    /// Lists detailed metadata (`KeyDetails`) for all keys in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<KeyDetails>`.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError>;

    /// Calculates and returns storage statistics based on the current index state.
    ///
    /// # Returns
    ///
    /// A `StorageStats` struct.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError>;

    /// Adds a single pad (address and key) to the free pad list.
    ///
    /// This method attempts to fetch the pad's current counter from storage before adding it.
    /// If fetching fails, the pad is not added, and a warning is logged.
    ///
    /// # Arguments
    ///
    /// * `address` - The address of the pad.
    /// * `key_bytes` - The secret key bytes of the pad.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures related to index modification.
    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError>;

    /// Adds multiple pads to the free pad list.
    ///
    /// Like `add_free_pad`, this attempts to fetch the counter for each pad before adding.
    /// Pads that fail the counter fetch are skipped with a warning.
    ///
    /// # Arguments
    ///
    /// * `pads` - A vector of tuples, each containing a pad's address and secret key bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures related to index modification.
    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;

    /// Takes (removes and returns) one pad from the free list, if available.
    ///
    /// # Returns
    ///
    /// `Ok(Some((address, key_bytes, counter)))` if a pad was available, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError>;

    /// Takes (removes and returns) all pads currently in the pending verification list.
    ///
    /// # Returns
    ///
    /// A `Vec` containing tuples of (address, key_bytes) for the removed pending pads.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError>;

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
    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError>;

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
    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError>;

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
    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError>;

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
    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    /// Returns a clone of the current in-memory `MasterIndex` state.
    ///
    /// # Returns
    ///
    /// A cloned `MasterIndex`.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures (e.g., mutex poisoning).
    async fn get_index_copy(&self) -> Result<MasterIndex, IndexError>;

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
    async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError>;

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
    async fn get_scratchpad_size(&self) -> Result<usize, IndexError>;

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
    async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
    ) -> Result<MasterIndex, IndexError>;

    /// Adds multiple pads to the pending verification list.
    ///
    /// # Arguments
    ///
    /// * `pads` - A vector of tuples, each containing a pad's address and secret key bytes.
    ///
    /// # Errors
    ///
    /// Returns `IndexError` on internal failures.
    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;
}

/// Default implementation of the `IndexManager` trait.
///
/// Manages the `MasterIndex` state within a `Mutex` and uses provided
/// `StorageManager` and `NetworkAdapter` implementations for persistence and remote operations.
/// Delegates most query and modification logic to functions in the `query` and `persistence` modules.
pub struct DefaultIndexManager {
    state: Mutex<MasterIndex>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultIndexManager {
    /// Creates a new `DefaultIndexManager`.
    ///
    /// Initializes with a default, empty `MasterIndex` state.
    /// The index should typically be loaded using `load_or_initialize` afterwards.
    ///
    /// # Arguments
    ///
    /// * `storage_manager` - An `Arc` reference to a `StorageManager` implementation.
    /// * `network_adapter` - An `Arc` reference to a `NetworkAdapter` implementation.
    ///
    /// # Returns
    ///
    /// A new `DefaultIndexManager` instance.
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

                if loaded_index.scratchpad_size == 0 {
                    warn!(
                        "Loaded index has scratchpad_size 0, setting to default: {}",
                        DEFAULT_SCRATCHPAD_SIZE
                    );
                    *state_guard = MasterIndex {
                        scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                        ..loaded_index
                    };
                } else {
                    *state_guard = loaded_index;
                }
                info!("Index loaded successfully from storage.");
                Ok(())
            }

            Err(IndexError::DeserializationError(e)) => {
                warn!(
                    "Failed to load or deserialize index from storage ({}). Initializing with default in memory.",
                    e
                );

                Ok(())
            }

            Err(IndexError::KeyNotFound(_)) => {
                warn!("Index key not found (unexpected persistence state?). Initializing with default in memory.");
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
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        debug!("IndexManager: Saving index to storage...");
        let state_guard = self.state.lock().await;
        save_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            _master_index_key,
            &state_guard,
        )
        .await
    }

    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_info_internal(&state_guard, key).cloned())
    }

    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::insert_key_info_internal(&mut state_guard, key, info)
    }

    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let mut state_guard = self.state.lock().await;
        let removed_info = query::remove_key_info_internal(&mut state_guard, key);

        if let Some(ref info) = removed_info {
            let mut pads_to_verify = Vec::new();

            let mut pads_to_free_directly = Vec::new();

            for pad_info in &info.pads {
                if let Some(pad_key) = info.pad_keys.get(&pad_info.address).cloned() {
                    match pad_info.status {
                        PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                            trace!(
                                "Pad {} for removed key '{}' is {:?}. Adding directly to free pool.",
                                pad_info.address,
                                key,
                                pad_info.status
                            );

                            pads_to_free_directly.push((pad_info.address, pad_key, 0u64));
                        }

                        PadStatus::Generated => {
                            trace!(
                                "Pad {} for removed key '{}' is {:?}. Adding to pending verification.",
                                pad_info.address,
                                key,
                                pad_info.status
                            );
                            pads_to_verify.push((pad_info.address, pad_key));
                        }
                    }
                } else {
                    warn!(
                        "Missing pad key for pad {} while removing key '{}'. Cannot release.",
                        pad_info.address, key
                    );
                }
            }

            if !pads_to_free_directly.is_empty() {
                debug!(
                    "Key '{}': Directly adding {} pads to the free list.",
                    key,
                    pads_to_free_directly.len()
                );

                query::add_free_pads_with_counters_internal(
                    &mut state_guard,
                    pads_to_free_directly,
                )?;
            }

            if !pads_to_verify.is_empty() {
                debug!(
                    "Key '{}': Adding {} pads to the pending verification list.",
                    key,
                    pads_to_verify.len()
                );

                query::add_pending_verification_pads_internal(&mut state_guard, pads_to_verify)?;
            }
        }

        Ok(removed_info)
    }

    async fn list_keys(&self) -> Result<Vec<String>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_keys_internal(&state_guard))
    }

    #[allow(dead_code)]
    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_details_internal(&state_guard, key))
    }

    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_all_key_details_internal(&state_guard))
    }

    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError> {
        let state_guard = self.state.lock().await;
        query::get_stats_internal(&state_guard)
    }

    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError> {
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
                    &mut state_guard,
                    address,
                    key_bytes,
                    counter,
                )
            }
            Err(e) => {
                warn!("Failed to fetch scratchpad to get counter for pad {} during add_free_pad: {}. Pad will not be added to free list.", address, e);

                Ok(())
            }
        }
    }

    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
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
                }
            }
        }

        if !pads_with_counters.is_empty() {
            let mut state_guard = self.state.lock().await;

            query::add_free_pads_with_counters_internal(&mut state_guard, pads_with_counters)?;
        }
        Ok(())
    }

    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_free_pad_internal(&mut state_guard))
    }

    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_pending_pads_internal(&mut state_guard))
    }

    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::remove_from_pending_internal(&mut state_guard, address_to_remove)
    }

    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::update_pad_status_internal(&mut state_guard, key, pad_address, new_status)
    }

    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::mark_key_complete_internal(&mut state_guard, key)
    }

    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Resetting index...");
        {
            let mut state_guard = self.state.lock().await;
            query::reset_index_internal(&mut state_guard);
        }

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
    }

    async fn get_scratchpad_size(&self) -> Result<usize, IndexError> {
        let state_guard = self.state.lock().await;
        let size = state_guard.scratchpad_size;
        if size == 0 {
            warn!("Index reports scratchpad size of 0, returning default.");
            Ok(DEFAULT_SCRATCHPAD_SIZE)
        } else {
            Ok(size)
        }
    }

    async fn fetch_remote(
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

    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::add_pending_pads_internal(&mut state_guard, pads)
    }
}
