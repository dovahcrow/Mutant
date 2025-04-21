use crate::api::init::initialize_layers;
use crate::api::ReserveCallback;
use crate::data::manager::DefaultDataManager;
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::{IndexEntry, MasterIndex};
use crate::internal_error::Error;
use crate::internal_events::{GetCallback, InitCallback, PurgeCallback, PutCallback};
use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use crate::types::{KeyDetails, KeySummary, MutAntConfig, StorageStats};
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use log::{debug, info, warn};
use std::collections::HashSet;
use std::sync::Arc;

/// The main entry point for interacting with the MutAnt distributed storage system.
///
/// This struct encapsulates the different managers (data, index, pad lifecycle) and the network adapter.
/// Instances are typically created using the `init` or `init_with_progress` associated functions.
#[derive(Clone)]
pub struct MutAnt {
    data_manager: Arc<DefaultDataManager>,
    pad_lifecycle_manager: Arc<DefaultPadLifecycleManager>,
    index_manager: Arc<DefaultIndexManager>,
    network_adapter: Arc<AutonomiNetworkAdapter>,
    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey,
}

impl MutAnt {
    /// Initializes a new MutAnt instance with default configuration.
    ///
    /// This is a convenience function that calls `init_with_progress` with default settings
    /// and no progress callback.
    ///
    /// # Arguments
    ///
    /// * `private_key_hex` - The user's private key as a hexadecimal string, used to derive
    ///   identity and encryption keys.
    ///
    /// # Errors
    ///
    /// Returns an `Error` if initialization fails at any layer (network, storage, index, etc.).
    pub async fn init(private_key_hex: String) -> Result<Self, Error> {
        Self::init_with_progress(private_key_hex, MutAntConfig::default(), None).await
    }

    /// Initializes a new MutAnt instance with custom configuration and progress reporting.
    ///
    /// This function sets up all necessary components: network adapter, storage backend,
    /// index manager, data manager, and pad lifecycle manager based on the provided configuration.
    ///
    /// # Arguments
    ///
    /// * `private_key_hex` - The user's private key as a hexadecimal string.
    /// * `config` - The `MutAntConfig` specifying settings like the target network.
    /// * `init_callback` - An optional callback (`InitCallback`) to receive progress updates
    ///   and potentially handle interactive prompts during initialization.
    ///
    /// # Errors
    ///
    /// Returns an `Error` if initialization fails, potentially due to invalid configuration,
    /// network issues, storage errors, or callback failures.
    pub async fn init_with_progress(
        private_key_hex: String,
        config: MutAntConfig,
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

    /// Initializes a new MutAnt instance configured only for fetching public data from Mainnet.
    ///
    /// This instance does **not** require a private key and cannot perform operations
    /// that require user identity or access to the private master index (e.g., store,
    /// fetch private, remove, update, list keys, purge).
    ///
    /// Its sole purpose is to allow calling `fetch_public` on Mainnet.
    /// Calling other methods on this instance will likely result in errors or panics.
    ///
    /// # Errors
    ///
    /// Returns an `Error` if network initialization fails.
    pub async fn init_public() -> Result<Self, Error> {
        info!("Initializing MutAnt public fetcher for Mainnet.");

        // Create a config for Mainnet
        let mut config = MutAntConfig::default();
        config.network = NetworkChoice::Mainnet;

        // Use a dummy, well-formatted private key hex just to satisfy the network adapter constructor.
        let dummy_key_hex = "4ef3b2bbdbc0727ad260f5449fe46972df63b1e03a6316cfbe0e2958eb8a91a6";

        // Initialize network adapter with the dummy key and Mainnet.
        let network_adapter_concrete = AutonomiNetworkAdapter::new(&dummy_key_hex, config.network)
            .map_err(|e| {
                Error::Config(format!(
                    "Failed network init for public fetcher (Mainnet): {}",
                    e
                ))
            })?;
        let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(network_adapter_concrete);
        info!("Public Fetcher: NetworkAdapter initialized for Mainnet.");

        let placeholder_master_key = SecretKey::from_hex(&dummy_key_hex)
            .map_err(|e| Error::Internal(format!("Failed create placeholder key: {:?}", e)))?;
        let placeholder_master_address =
            ScratchpadAddress::new(placeholder_master_key.public_key());

        // Initialize managers with the network adapter and placeholder keys.
        let index_manager = Arc::new(DefaultIndexManager::new(
            Arc::clone(&network_adapter),
            placeholder_master_key.clone(),
        ));

        let pad_lifecycle_manager = Arc::new(DefaultPadLifecycleManager::new(
            Arc::clone(&index_manager),
            Arc::clone(&network_adapter),
        ));

        let data_manager = Arc::new(DefaultDataManager::new(
            Arc::clone(&network_adapter),
            Arc::clone(&index_manager),
            Arc::clone(&pad_lifecycle_manager),
        ));
        info!("Public Fetcher: Managers initialized with placeholders (Mainnet).");

        Ok(Self {
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address: placeholder_master_address,
            master_index_key: placeholder_master_key,
        })
    }

    /// Initializes a new MutAnt instance configured only for fetching public data from Devnet.
    ///
    /// Similar to `init_public` but connects to the local Devnet.
    /// Requires `--local` flag in the CLI.
    ///
    /// # Errors
    ///
    /// Returns an `Error` if network initialization fails.
    pub async fn init_public_local() -> Result<Self, Error> {
        info!("Initializing MutAnt public fetcher for Devnet.");

        // Create a config for Devnet
        let mut config = MutAntConfig::default();
        config.network = NetworkChoice::Devnet;

        // Generate a random, ephemeral private key just to satisfy the wallet constructor.
        let dummy_key_hex = "4ef3b2bbdbc0727ad260f5449fe46972df63b1e03a6316cfbe0e2958eb8a91a6";

        // Initialize network adapter with the random key and Devnet.
        let network_adapter_concrete = AutonomiNetworkAdapter::new(&dummy_key_hex, config.network)
            .map_err(|e| {
                Error::Config(format!(
                    "Failed network init for public fetcher (Devnet): {}",
                    e
                ))
            })?;
        let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(network_adapter_concrete);
        info!("Public Fetcher: NetworkAdapter initialized for Devnet.");

        // Use placeholder keys and address for the master index.
        let placeholder_master_key = SecretKey::from_hex(&dummy_key_hex)
            .map_err(|e| Error::Internal(format!("Failed create placeholder key: {:?}", e)))?;
        let placeholder_master_address =
            ScratchpadAddress::new(placeholder_master_key.public_key());

        // Initialize managers with the network adapter and placeholder keys.
        let index_manager = Arc::new(DefaultIndexManager::new(
            Arc::clone(&network_adapter),
            placeholder_master_key.clone(),
        ));
        let pad_lifecycle_manager = Arc::new(DefaultPadLifecycleManager::new(
            Arc::clone(&index_manager),
            Arc::clone(&network_adapter),
        ));
        let data_manager = Arc::new(DefaultDataManager::new(
            Arc::clone(&network_adapter),
            Arc::clone(&index_manager),
            Arc::clone(&pad_lifecycle_manager),
        ));
        info!("Public Fetcher: Managers initialized with placeholders (Devnet).");

        Ok(Self {
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address: placeholder_master_address,
            master_index_key: placeholder_master_key,
        })
    }

    /// Stores data under a specific key.
    ///
    /// This is a convenience function that calls `store_with_progress` with no progress callback.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The unique key to associate with the data.
    /// * `data_bytes` - The data to be stored as a byte slice.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the underlying data manager fails to store the data.
    pub async fn store(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::store called for key '{}'", user_key);
        self.data_manager
            .store(user_key, data_bytes, None)
            .await
            .map_err(Error::Data)
    }

    /// Stores data under a specific key with progress reporting.
    ///
    /// This method handles chunking the data, reserving necessary storage pads,
    /// writing chunks, confirming writes, and updating the index.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The unique key to associate with the data.
    /// * `data_bytes` - The data to be stored as a byte slice.
    /// * `callback` - An optional callback (`PutCallback`) to receive progress updates (`PutEvent`).
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the storage operation fails. Also implicitly saves the index cache,
    /// logging a warning if that fails.
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

        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after store operation: {}", e);
        }

        Ok(())
    }

    /// Fetches data associated with a specific key.
    ///
    /// This is a convenience function that calls `fetch_with_progress` with no progress callback.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key of the data to retrieve.
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the retrieved data.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the key is not found or if fetching fails.
    pub async fn fetch(&self, user_key: &str) -> Result<Vec<u8>, Error> {
        debug!("MutAnt::fetch called for key '{}'", user_key);
        self.data_manager
            .fetch(user_key, None)
            .await
            .map_err(Error::Data)
    }

    /// Fetches data associated with a specific key with progress reporting.
    ///
    /// This method handles locating the data chunks, fetching them from storage,
    /// and reassembling them.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key of the data to retrieve.
    /// * `callback` - An optional callback (`GetCallback`) to receive progress updates (`GetEvent`).
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the retrieved data.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the key is not found or if fetching fails.
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

    /// Removes data associated with a specific key.
    ///
    /// This method removes the key entry from the index and marks the associated storage pads
    /// as potentially free (actual reclamation might happen during a `purge` operation).
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key of the data to remove.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the key is not found or removal fails. Also implicitly saves the index cache,
    /// logging a warning if that fails.
    pub async fn remove(&self, user_key: &str) -> Result<(), Error> {
        debug!("MutAnt::remove called for key '{}'", user_key);
        self.data_manager
            .remove(user_key)
            .await
            .map_err(Error::Data)?;

        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after remove operation: {}", e);
        }

        Ok(())
    }

    /// Updates the data associated with an existing key.
    ///
    /// This is conceptually similar to a `remove` followed by a `store`.
    /// It's a convenience function calling `update_with_progress` with no callback.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key of the data to update.
    /// * `data_bytes` - The new data to be stored.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the update operation fails.
    pub async fn update(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::update called for key '{}'", user_key);
        self.data_manager
            .update(user_key, data_bytes, None)
            .await
            .map_err(Error::Data)
    }

    /// Updates the data associated with an existing key, with progress reporting.
    ///
    /// Handles removing the old data reference and storing the new data.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key of the data to update.
    /// * `data_bytes` - The new data to be stored.
    /// * `callback` - An optional callback (`PutCallback`) for the storage part of the update.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the update operation fails.
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

    /// Updates the data associated with an existing public upload name.
    ///
    /// This operation overwrites the content of the existing public index scratchpad
    /// to point to newly uploaded data chunks. The original data chunks become orphaned.
    /// The public index address remains the same.
    /// This is a convenience function calling `update_public_with_progress` with no callback.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the public upload to update.
    /// * `data_bytes` - The new data to store.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the key is not found, if it's a private key, or if the update fails.
    /// Returns `Error::PadLifecycle` if saving the index cache fails afterwards.
    pub async fn update_public(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::update_public called for name '{}'", name);
        self.update_public_with_progress(name, data_bytes, None)
            .await
    }

    /// Updates the data associated with an existing public upload name, with progress reporting.
    ///
    /// This operation overwrites the content of the existing public index scratchpad
    /// to point to newly uploaded data chunks. The original data chunks become orphaned.
    /// The public index address remains the same.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the public upload to update.
    /// * `data_bytes` - The new data to store.
    /// * `callback` - An optional callback (`PutCallback`) for progress reporting.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the key is not found, if it's a private key, or if the update fails.
    /// Returns `Error::PadLifecycle` if saving the index cache fails afterwards (logged as warning).
    pub async fn update_public_with_progress(
        &self,
        name: &str,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!(
            "MutAnt::update_public_with_progress started for name '{}'",
            name
        );

        // Delegate the core logic to the data manager
        let result = self
            .data_manager
            .update_public(name, data_bytes, callback)
            .await
            .map_err(Error::Data);

        // Attempt to save the index cache regardless of the update result
        let network_choice = self.network_adapter.get_network_choice();
        if let Err(e) = self
            .pad_lifecycle_manager
            .save_index_cache(network_choice)
            .await
        {
            warn!(
                "Failed to save index cache after update_public_with_progress operation for '{}': {}",
                name, e
            );
            // If the update itself was successful, return the index save error.
            if result.is_ok() {
                // Map PadLifecycleError to crate::Error::PadLifecycle
                return Err(Error::PadLifecycle(e));
            }
        }

        result // Return the original result from data_manager.update_public
    }

    /// Lists summarized information for all user keys and public uploads stored in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<KeySummary>` containing the name, public status, and address (if public) for each entry.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if retrieving the key list fails.
    pub async fn list_keys(&self) -> Result<Vec<KeySummary>, Error> {
        debug!("MutAnt::list_keys called");
        self.index_manager.list_keys().await.map_err(Error::Index)
    }

    /// Retrieves detailed metadata (`KeyDetails`) for a specific user key or public upload name.
    ///
    /// # Arguments
    ///
    /// * `user_key_or_name` - The user key or public upload name.
    ///
    /// # Returns
    ///
    /// `Ok(Some(KeyDetails))` if the key/name exists, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if retrieving the key details fails.
    pub async fn get_key_details(
        &self,
        user_key_or_name: &str,
    ) -> Result<Option<KeyDetails>, Error> {
        debug!(
            "MutAnt::get_key_details called for key/name '{}'",
            user_key_or_name
        );
        self.index_manager
            .get_key_details(user_key_or_name)
            .await
            .map_err(Error::Index)
    }

    /// Lists detailed information for all keys stored in the index.
    ///
    /// # Returns
    ///
    /// A `Vec<KeyDetails>` containing metadata for each key.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if retrieving the key details fails.
    pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error> {
        debug!("MutAnt::list_key_details called");
        self.index_manager
            .list_all_key_details()
            .await
            .map_err(Error::Index)
    }

    /// Retrieves detailed statistics about the storage backend.
    ///
    /// # Returns
    ///
    /// A `StorageStats` struct containing various metrics about pad usage and space.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if retrieving the statistics fails.
    pub async fn get_storage_stats(&self) -> Result<StorageStats, Error> {
        debug!("MutAnt::get_storage_stats called");
        self.index_manager
            .get_storage_stats()
            .await
            .map_err(Error::Index)
    }

    /// Imports an externally generated pad (represented by its private key) into the free pad pool.
    ///
    /// This allows incorporating pads generated elsewhere into the current MutAnt instance.
    ///
    /// # Arguments
    ///
    /// * `private_key_hex` - The private key of the pad to import, as a hexadecimal string.
    ///
    /// # Errors
    ///
    /// Returns `Error::PadLifecycle` if importing the pad fails (e.g., invalid key, duplicate).
    pub async fn import_free_pad(&self, private_key_hex: &str) -> Result<(), Error> {
        debug!("MutAnt::import_free_pad called");
        self.pad_lifecycle_manager
            .import_external_pad(private_key_hex)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Explicitly reserves a specified number of free pads for future use.
    ///
    /// While `store` operations automatically reserve pads, this allows pre-reserving.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of pads to reserve.
    /// * `callback` - An optional callback (`ReserveCallback`) to receive progress updates.
    ///
    /// # Returns
    ///
    /// The number of pads successfully reserved.
    ///
    /// # Errors
    ///
    /// Returns `Error::PadLifecycle` if the reservation process fails.
    pub async fn reserve_pads(
        &self,
        count: usize,
        callback: Option<ReserveCallback>,
    ) -> Result<usize, Error> {
        debug!("MutAnt::reserve_pads called, delegating to PadLifecycleManager");
        self.pad_lifecycle_manager
            .reserve_pads(count, callback)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Performs a storage purge operation.
    ///
    /// This typically involves verifying the integrity of stored data, potentially reclaiming
    /// space from deleted or corrupted entries, and ensuring consistency between the index and storage.
    ///
    /// # Arguments
    ///
    /// * `callback` - An optional callback (`PurgeCallback`) to receive progress updates.
    ///
    /// # Errors
    ///
    /// Returns `Error::PadLifecycle` if the purge operation encounters errors.
    pub async fn purge(&self, callback: Option<PurgeCallback>) -> Result<(), Error> {
        debug!("MutAnt::purge called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .purge(callback, network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Persists the current state of the master index to the network/storage.
    ///
    /// The master index holds the primary mapping of user keys to data locations.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if saving the master index fails.
    pub async fn save_master_index(&self) -> Result<(), Error> {
        debug!("MutAnt::save_master_index called");
        self.index_manager
            .save(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    /// Resets the local index state, potentially by fetching the latest version from the network.
    ///
    /// This can be used to recover from a corrupted local index or to synchronize with updates.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if the reset operation fails.
    pub async fn reset(&self) -> Result<(), Error> {
        debug!("MutAnt::reset called");
        self.index_manager
            .reset(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    /// Saves the internal cache related to pad lifecycle management (e.g., free pad list).
    ///
    /// This is often called implicitly after operations that modify pad state (`store`, `remove`).
    ///
    /// # Errors
    ///
    /// Returns `Error::PadLifecycle` if saving the cache fails.
    pub async fn save_index_cache(&self) -> Result<(), Error> {
        debug!("MutAnt::save_index_cache called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .save_index_cache(network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    /// Fetches the master index directly from its remote storage location.
    ///
    /// This bypasses local caching and retrieves the index based on its known address.
    ///
    /// # Returns
    ///
    /// The fetched `MasterIndex`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if fetching the remote index fails.
    pub async fn fetch_remote_master_index(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::fetch_remote_master_index called");

        self.index_manager
            .fetch_remote(&self.master_index_address)
            .await
            .map_err(Error::Index)
    }

    /// Updates the in-memory master index with a provided `MasterIndex` instance.
    ///
    /// This is typically used after fetching a remote index or performing merge operations.
    /// Note: This usually does *not* automatically persist the change; `save_master_index` might be needed.
    ///
    /// # Arguments
    ///
    /// * `new_index` - The `MasterIndex` instance to adopt.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if updating the internal index representation fails.
    pub async fn update_internal_master_index(&self, new_index: MasterIndex) -> Result<(), Error> {
        debug!("MutAnt::update_internal_master_index called");
        self.index_manager
            .update_index(new_index)
            .await
            .map_err(Error::Index)
    }

    /// Returns a clone of the current in-memory master index.
    ///
    /// Useful for inspection or operations that require read-only access to the index state.
    ///
    /// # Returns
    ///
    /// A `MasterIndex` clone.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if cloning the index fails.
    pub async fn get_index_copy(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::get_index_copy called");
        self.index_manager
            .get_index_copy()
            .await
            .map_err(Error::Index)
    }

    /// Gets the currently configured network choice (e.g., Mainnet, Testnet).
    pub fn get_network_choice(&self) -> NetworkChoice {
        self.network_adapter.get_network_choice()
    }

    /// Stores data publicly under a specified name.
    ///
    /// The data is NOT encrypted. A unique secret key is generated for this upload
    /// to sign the data and index scratchpads, but this key is stored (in byte form)
    /// in the master index.
    ///
    /// This method chunks the data, uploads the chunks and a public index, and updates
    /// the master index.
    ///
    /// # Arguments
    ///
    /// * `name` - A unique name to identify this public upload.
    /// * `data_bytes` - The raw data to store.
    /// * `callback` - An optional callback for progress reporting (`PutEvent`).
    ///
    /// # Returns
    ///
    /// The `ScratchpadAddress` of the public index scratchpad.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if the underlying storage operation fails (e.g., name collision, network error).
    /// Returns `Error::Index` if saving the index cache fails afterwards.
    pub async fn store_public(
        &self,
        name: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        debug!("MutAnt::store_public started for name '{}'", name);
        let result = self
            .data_manager
            .store_public(name.clone(), data_bytes, callback)
            .await
            .map_err(Error::Data);

        // Regardless of success/failure of the data store, try to save the index
        // (the index manager might have been updated even if data store failed partially)
        if let Err(e) = self.save_index_cache().await {
            warn!(
                "Failed to save index cache after store_public operation for '{}': {}",
                name, e
            );
            // Don't shadow the original store_public error if there was one
            if result.is_ok() {
                return Err(e); // If store_public was ok, return the index save error
            }
        }

        result // Return the original result from store_public
    }

    /// Retrieves the public index `ScratchpadAddress` for a given public upload name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the public upload.
    ///
    /// # Returns
    ///
    /// `Ok(Some(ScratchpadAddress))` if the public upload exists, `Ok(None)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `Error::Index` if there's an issue accessing the index.
    pub async fn get_public_address(&self, name: &str) -> Result<Option<ScratchpadAddress>, Error> {
        debug!("MutAnt::get_public_address for name '{}'", name);
        let index_copy = self.index_manager.get_index_copy().await?;
        Ok(index_copy.index.get(name).and_then(|entry| match entry {
            IndexEntry::PublicUpload(info) => Some(info.index_pad.address),
            IndexEntry::PrivateKey(_) => None, // Name exists but is a private key
        }))
    }

    /// Fetches publicly stored data using its index scratchpad address.
    ///
    /// This function is static relative to MutAnt instance data (doesn't use `&self` directly
    /// for master index/key) but requires a network client.
    /// It delegates to the underlying `DataManager::fetch_public`.
    ///
    /// # Arguments
    ///
    /// * `client` - An `autonomi::Client` instance to interact with the network.
    ///            *(Note: This API might change; ideally, we'd use the internal adapter)*
    /// * `public_index_address` - The address returned by `store_public` or `get_public_address`.
    /// * `callback` - An optional callback for progress reporting (`GetEvent`).
    ///
    /// # Returns
    ///
    /// A `Bytes` object containing the reassembled public data.
    ///
    /// # Errors
    ///
    /// Returns `Error::Data` if fetching fails (e.g., invalid address, network error, signature mismatch).
    pub async fn fetch_public(
        &self,
        public_index_address: ScratchpadAddress,
        callback: Option<GetCallback>,
    ) -> Result<Bytes, Error> {
        debug!(
            "MutAnt::fetch_public called for address {}",
            public_index_address
        );
        self.data_manager
            .fetch_public(public_index_address, callback)
            .await
            .map_err(Error::Data)
    }

    /// Returns a set of all currently occupied scratchpad addresses used for private data.
    ///
    /// This is useful for external tools or analysis to understand which pads are in active use.
    ///
    pub async fn get_occupied_private_pad_addresses(
        &self,
    ) -> Result<HashSet<ScratchpadAddress>, Error> {
        debug!("MutAnt::get_occupied_private_pad_addresses called");
        self.index_manager
            .get_occupied_private_pad_addresses()
            .await
            .map_err(Error::Index)
    }
}
