use crate::api::init::initialize_layers; // Keep this specific import
use crate::api::ReserveCallback; // Removed unused ReserveEvent
use crate::data::manager::DataManager;
use crate::error::Error;
use crate::index::manager::IndexManager;
use crate::index::structure::MasterIndex; // Keep specific structure import
use crate::network::{NetworkAdapter, NetworkChoice}; // Removed unused NetworkError
use crate::pad_lifecycle::PadLifecycleManager; // Removed unused PadOrigin
use crate::types::MutAntConfig; // Import MutAntConfig
use crate::{GetCallback, InitCallback, KeyDetails, PurgeCallback, PutCallback, StorageStats};
use autonomi::{ScratchpadAddress, SecretKey}; // Keep for now, might remove Scratchpad later
use log::{debug, info, warn}; // Removed unused error, trace
use std::sync::Arc;

/// The main public structure for interacting with the MutAnt library.
/// Provides methods for storing, fetching, managing, and querying data.
#[derive(Clone)] // Clone is cheap due to Arcs
pub struct MutAnt {
    // Keep Arcs to the managers for delegation
    data_manager: Arc<dyn DataManager>,
    pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
    index_manager: Arc<dyn IndexManager>,
    network_adapter: Arc<dyn NetworkAdapter>, // Needed for get_network_choice

    // Store master index info needed for some operations (save, reset)
    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey, // Cloning SecretKey is cheap
}

impl MutAnt {
    /// Initializes the MutAnt library with default configuration.
    /// Use `init_with_progress` for custom configuration and progress reporting.
    ///
    /// # Arguments
    /// * `private_key_hex` - The user's private key in hexadecimal format (with or without "0x" prefix).
    pub async fn init(private_key_hex: String) -> Result<Self, Error> {
        Self::init_with_progress(private_key_hex, MutAntConfig::default(), None).await
    }

    /// Initializes the MutAnt library with custom configuration and optional progress reporting.
    ///
    /// # Arguments
    /// * `private_key_hex` - The user's private key in hexadecimal format.
    /// * `config` - The `MutAntConfig` specifying network choice, etc.
    /// * `init_callback` - An optional callback function to receive `InitProgressEvent` updates.
    pub async fn init_with_progress(
        private_key_hex: String,
        config: MutAntConfig, // Take config by value
        init_callback: Option<InitCallback>,
    ) -> Result<Self, Error> {
        info!("MutAnt::init_with_progress started.");
        let (
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address,
            master_index_key,
        ) = initialize_layers(&private_key_hex, &config, init_callback).await?;

        Ok(Self {
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address,
            master_index_key,
        })
    }

    // --- Core Data Operations ---

    /// Stores raw bytes under a given user key.
    /// This is an alias for `store_with_progress` without a callback.
    pub async fn store(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::store called for key '{}'", user_key);
        self.data_manager
            .store(user_key, data_bytes, None)
            .await
            .map_err(Error::Data) // Map DataError to top-level Error::Data
    }

    /// Stores raw bytes under a given user key with progress reporting.
    pub async fn store_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!(
            "MutAnt::store_with_progress called for key \'{}\'",
            user_key
        );
        self.data_manager
            .store(user_key, data_bytes, callback)
            .await
            .map_err(Error::Data)?;

        // After successful store, save the updated index to cache
        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after store operation: {}", e);
            // Do not return error, as store itself succeeded.
        }

        Ok(())
    }

    /// Fetches the raw bytes associated with the given user key.
    /// Alias for `fetch_with_progress` without a callback.
    pub async fn fetch(&self, user_key: &str) -> Result<Vec<u8>, Error> {
        debug!("MutAnt::fetch called for key '{}'", user_key);
        self.data_manager
            .fetch(user_key, None)
            .await
            .map_err(Error::Data)
    }

    /// Fetches the raw bytes associated with the given user key, with progress reporting.
    pub async fn fetch_with_progress(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        debug!("MutAnt::fetch_with_progress called for key '{}'", user_key);
        self.data_manager
            .fetch(user_key, callback)
            .await
            .map_err(Error::Data)
    }

    /// Removes a user key and its associated data/metadata.
    /// Note: Pad release might be incomplete due to key management limitations (see DataOps::remove).
    pub async fn remove(&self, user_key: &str) -> Result<(), Error> {
        debug!("MutAnt::remove called for key '{}'", user_key);
        self.data_manager
            .remove(user_key)
            .await
            .map_err(Error::Data)?; // Propagate error if remove fails

        // After successful remove, save the updated index to cache
        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after remove operation: {}", e);
            // Do not return error, as remove itself succeeded in memory.
        }

        Ok(())
    }

    /// Updates the raw bytes associated with an existing user key.
    /// Alias for `update_with_progress` without a callback.
    /// Returns `Error::Data(DataError::KeyNotFound)` if the key does not exist.
    /// Note: Pad release might be incomplete due to key management limitations (see DataOps::update).
    pub async fn update(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::update called for key '{}'", user_key);
        self.data_manager
            .update(user_key, data_bytes, None)
            .await
            .map_err(Error::Data)
    }

    /// Updates the raw bytes associated with an existing user key, with progress reporting.
    /// Returns `Error::Data(DataError::KeyNotFound)` if the key does not exist.
    /// Note: Pad release might be incomplete due to key management limitations (see DataOps::update).
    pub async fn update_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!("MutAnt::update_with_progress called for key '{}'", user_key);
        self.data_manager
            .update(user_key, data_bytes, callback)
            .await
            .map_err(Error::Data)
    }

    // --- Querying & Statistics ---

    /// Retrieves a list of all user keys currently tracked.
    pub async fn list_keys(&self) -> Result<Vec<String>, Error> {
        debug!("MutAnt::list_keys called");
        self.index_manager.list_keys().await.map_err(Error::Index)
    }

    /// Retrieves detailed information (`KeyDetails`) for all user keys.
    pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error> {
        debug!("MutAnt::list_key_details called");
        self.index_manager
            .list_all_key_details()
            .await
            .map_err(Error::Index)
    }

    /// Retrieves detailed statistics about the storage usage.
    pub async fn get_storage_stats(&self) -> Result<StorageStats, Error> {
        debug!("MutAnt::get_storage_stats called");
        self.index_manager
            .get_storage_stats()
            .await
            .map_err(Error::Index)
    }

    // --- Pad & Index Management ---

    /// Imports a free scratchpad using its private key hex string.
    /// The caller should ideally call `save_master_index` afterwards if persistence is desired immediately.
    pub async fn import_free_pad(&self, private_key_hex: &str) -> Result<(), Error> {
        debug!("MutAnt::import_free_pad called");
        self.pad_lifecycle_manager
            .import_external_pad(private_key_hex)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Reserves multiple new scratchpads concurrently, saves the index incrementally,
    /// and provides progress updates via a callback.
    ///
    /// # Arguments
    /// * `count` - The number of pads to reserve.
    /// * `callback` - An optional callback to receive `ReserveEvent` updates.
    ///
    /// # Returns
    /// The number of pads successfully reserved and saved to the index cache.
    pub async fn reserve_pads(
        &self,
        count: usize,
        callback: Option<ReserveCallback>,
    ) -> Result<usize, Error> {
        debug!("MutAnt::reserve_pads called, delegating to PadLifecycleManager");
        self.pad_lifecycle_manager
            .reserve_pads(count, callback)
            .await
            .map_err(Error::PadLifecycle) // Map PadLifecycleError to top-level Error
    }

    /// Verifies pads in the pending list against the network.
    /// Moves verified pads to the free list. Discards invalid ones.
    /// Saves the updated index state ONLY to the local cache.
    pub async fn purge(&self, callback: Option<PurgeCallback>) -> Result<(), Error> {
        debug!("MutAnt::purge called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .purge(callback, network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Saves the current in-memory master index to the remote backend (network storage).
    /// This should be called after operations that modify the index significantly (store, remove, update, import)
    /// if immediate persistence is required.
    pub async fn save_master_index(&self) -> Result<(), Error> {
        debug!("MutAnt::save_master_index called");
        self.index_manager
            .save(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    /// Resets the master index (both in-memory and remotely) to its initial empty state.
    /// WARNING: This is a destructive operation and will orphan existing data pads.
    pub async fn reset(&self) -> Result<(), Error> {
        debug!("MutAnt::reset called");
        self.index_manager
            .reset(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    // --- Advanced/Sync Operations ---

    /// Saves the current in-memory index state to the local filesystem cache.
    pub async fn save_index_cache(&self) -> Result<(), Error> {
        debug!("MutAnt::save_index_cache called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .save_index_cache(network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Fetches the MasterIndex directly from the remote backend, bypassing the local cache.
    /// Useful for synchronization purposes.
    pub async fn fetch_remote_master_index(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::fetch_remote_master_index called");
        // Call the fetch_remote method on the IndexManager
        self.index_manager
            .fetch_remote(&self.master_index_address)
            .await
            .map_err(Error::Index)
    }

    /// Updates the internal, in-memory master index directly with the provided structure.
    /// Does NOT persist the change automatically. Call `save_master_index` afterwards if needed.
    /// Useful for synchronization purposes after fetching a remote index.
    pub async fn update_internal_master_index(&self, new_index: MasterIndex) -> Result<(), Error> {
        debug!("MutAnt::update_internal_master_index called");
        self.index_manager
            .update_index(new_index)
            .await
            .map_err(Error::Index)
    }

    /// Returns a clone of the current in-memory MasterIndex state.
    /// Useful for synchronization or backup, use with caution.
    pub async fn get_index_copy(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::get_index_copy called");
        self.index_manager
            .get_index_copy()
            .await
            .map_err(Error::Index)
    }

    // --- Utility ---

    /// Retrieves the network choice (Devnet or Mainnet) this MutAnt instance is configured for.
    pub fn get_network_choice(&self) -> NetworkChoice {
        self.network_adapter.get_network_choice()
    }
}
