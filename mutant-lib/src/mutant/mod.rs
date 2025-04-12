use crate::error::Error;
use crate::events::{GetCallback, InitCallback, PutCallback};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::pad_manager::PadManager;
use crate::storage::Storage;
use autonomi::{ScratchpadAddress, Wallet};
use chrono::{DateTime, Utc};
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub mod data_structures;
pub mod remove_logic;
pub mod store_logic;
pub mod update_logic;

// --- Internal Keys for Management Data ---
pub const MASTER_INDEX_KEY: &str = "__MUTANT_MASTER_INDEX__";
pub const FREE_DIRECT_LIST_KEY: &str = "__mutant_free_direct_v1__";
pub const FREE_CHUNK_LIST_KEY: &str = "__mutant_free_chunk_v1__";

/// Actual physical size of scratchpads on the network.
pub(crate) const PHYSICAL_SCRATCHPAD_SIZE: usize = 4 * 1024 * 1024; // 4MB
/// Usable size within a scratchpad, leaving margin for network overhead/encryption.
/// NOTE: This should be the value persisted in MasterIndexStorage.scratchpad_size
pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = PHYSICAL_SCRATCHPAD_SIZE - 512; // 4MB minus 512 bytes margin

/// Holds calculated storage statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageStats {
    /// The configured size of each individual scratchpad in bytes.
    pub scratchpad_size: usize,
    /// The total number of scratchpads managed (occupied + free).
    pub total_pads: usize,
    /// The number of scratchpads currently holding data.
    pub occupied_pads: usize,
    /// The number of scratchpads available for reuse.
    pub free_pads: usize,
    /// The total storage capacity across all managed scratchpads (total_pads * scratchpad_size).
    pub total_space_bytes: u64,
    /// The storage capacity currently used by occupied scratchpads (occupied_pads * scratchpad_size).
    pub occupied_pad_space_bytes: u64,
    /// The storage capacity currently available in free scratchpads (free_pads * scratchpad_size).
    pub free_pad_space_bytes: u64,
    /// The actual total size of the data stored across all occupied scratchpads.
    pub occupied_data_bytes: u64,
    /// The difference between the space allocated by occupied pads and the actual data stored (internal fragmentation).
    pub wasted_space_bytes: u64,
}

/// Public struct to hold details for the `ls -l` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDetails {
    pub key: String,
    pub size: usize,
    pub modified: DateTime<Utc>,
}

/// Manages raw byte data storage using the PadManager.
#[derive(Clone)]
pub struct MutAnt {
    pub(crate) storage: Arc<Storage>,
    master_index_addr: ScratchpadAddress,
    master_index_storage: Arc<Mutex<MasterIndexStorage>>,
    pub(crate) pad_manager: PadManager,
}

impl MutAnt {
    /// Creates a new MutAnt instance without progress reporting.
    /// Use `init_with_progress` for detailed initialization feedback.
    pub async fn init(
        wallet: Wallet,
        private_key_hex: String,
    ) -> Result<(Self, Option<JoinHandle<()>>), Error> {
        Self::init_with_progress(wallet, private_key_hex, None).await
    }

    /// Creates a new MutAnt instance with optional progress reporting via callback.
    pub async fn init_with_progress(
        wallet: Wallet,
        private_key_hex: String,
        mut init_callback: Option<InitCallback>,
    ) -> Result<(Self, Option<JoinHandle<()>>), Error> {
        info!("Initializing MutAnt...");

        {
            info!("Initializing Storage layer...");

            let (storage_instance, storage_init_handle, mis_mutex): (
                Storage,
                Option<JoinHandle<()>>,
                Arc<Mutex<MasterIndexStorage>>,
            ) = match crate::storage::new(wallet, private_key_hex, &mut init_callback).await {
                Ok(result) => result,
                Err(e) => {
                    return Err(e);
                }
            };
            let storage_arc = Arc::new(storage_instance);
            info!("Storage layer initialized.");

            info!("Initializing PadManager...");
            let pad_manager = PadManager::new(storage_arc.clone(), mis_mutex.clone());
            info!("PadManager initialized.");

            let mis_addr_from_storage = storage_arc.get_master_index_info().0;

            let mutant = Self {
                storage: storage_arc,
                pad_manager,
                master_index_storage: mis_mutex,
                master_index_addr: mis_addr_from_storage,
            };

            info!("MutAnt initialization complete.");
            Ok((mutant, storage_init_handle))
        }
    }

    /// Stores raw bytes under a given user key without progress reporting.
    /// Use `store_with_progress` for detailed feedback.
    pub async fn store(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        self.store_with_progress(user_key, data_bytes, None).await
    }

    /// Stores raw bytes under a given user key with optional progress reporting via callback.
    pub async fn store_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        if user_key == MASTER_INDEX_KEY {
            return Err(Error::InvalidInput(
                "User key conflicts with internal master index key".to_string(),
            ));
        }
        store_logic::store_item(self, user_key, data_bytes, callback).await
    }

    /// Fetches the raw bytes associated with the given user key without progress reporting.
    /// Use `fetch_with_progress` for detailed feedback.
    pub async fn fetch(&self, key: &str) -> Result<Vec<u8>, Error> {
        self.fetch_with_progress(key, None).await
    }

    /// Fetches the raw bytes associated with the given user key, with optional progress reporting.
    pub async fn fetch_with_progress(
        &self,
        key: &str,
        mut callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        info!("MutAnt: fetch called for key '{}'", key);
        self.pad_manager.retrieve_data(key, &mut callback).await
    }

    /// Removes a user key and its associated data/metadata.
    pub async fn remove(&self, user_key: &str) -> Result<(), Error> {
        debug!("MutAnt::remove: Entered for key '{}'", user_key);
        if user_key == MASTER_INDEX_KEY {
            error!("Attempted to remove the master index key directly. Ignoring.");
            return Ok(());
        }
        debug!(
            "MutAnt::remove: Calling remove_logic::delete_item for key '{}'",
            user_key
        );
        let result = remove_logic::delete_item(self, user_key).await;
        debug!(
            "MutAnt::remove: Returned from remove_logic::delete_item for key '{}'",
            user_key
        );
        result
    }

    /// Updates the raw bytes associated with an existing user key without progress reporting.
    /// Use `update_with_progress` for detailed feedback.
    ///
    /// Returns `Error::KeyNotFound` if the key does not exist.
    pub async fn update(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        self.update_with_progress(user_key, data_bytes, None).await
    }

    /// Updates the raw bytes associated with an existing user key, with optional progress reporting.
    ///
    /// Returns `Error::KeyNotFound` if the key does not exist.
    pub async fn update_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!("MutAnt::update: Entered for key '{}'", user_key);
        if user_key == MASTER_INDEX_KEY {
            error!("Attempted to update the master index key directly. Denying access.");
            return Err(Error::OperationNotSupported);
        }
        let result = update_logic::update_item(self, user_key.clone(), data_bytes, callback).await;
        debug!(
            "MutAnt::update: Returned from update_logic::update_item for key '{}'",
            user_key
        );
        result
    }

    /// Retrieves a list of all user keys.
    pub async fn get_user_keys(&self) -> Vec<String> {
        let mis_guard = self.master_index_storage.lock().await;
        let all_user_keys: Vec<String> = mis_guard
            .index
            .keys()
            .filter(|k| *k != MASTER_INDEX_KEY)
            .cloned()
            .collect();
        drop(mis_guard);
        all_user_keys
    }

    /// Retrieves detailed statistics about the storage usage.
    pub async fn get_storage_stats(&self) -> Result<StorageStats, Error> {
        debug!("Getting storage stats...");
        let mis_guard = self.master_index_storage.lock().await;
        debug!("Master index lock acquired for stats.");

        let scratchpad_size = mis_guard.scratchpad_size;
        if scratchpad_size == 0 {
            error!("Cannot calculate stats: Scratchpad size is zero.");
            return Err(Error::InternalError(
                "Cannot calculate stats with zero scratchpad size".to_string(),
            ));
        }

        let free_pads_count = mis_guard.free_pads.len();
        let mut occupied_pads_count = 0;
        let mut occupied_data_size_total: u64 = 0;

        for key_info in mis_guard.index.values() {
            occupied_pads_count += key_info.pads.len();
            occupied_data_size_total += key_info.data_size as u64;
        }

        let total_pads_count = occupied_pads_count + free_pads_count;

        let scratchpad_size_u64 = scratchpad_size as u64;
        let occupied_pad_space_bytes = occupied_pads_count as u64 * scratchpad_size_u64;
        let free_pad_space_bytes = free_pads_count as u64 * scratchpad_size_u64;
        let total_space_bytes = total_pads_count as u64 * scratchpad_size_u64;

        let wasted_space_bytes = occupied_pad_space_bytes.saturating_sub(occupied_data_size_total);

        debug!(
            "Stats calculated: total_pads={}, occupied_pads={}, free_pads={}, total_space={}, data_size={}",
            total_pads_count, occupied_pads_count, free_pads_count, total_space_bytes, occupied_data_size_total
        );
        debug!("Master index lock released after stats.");

        Ok(StorageStats {
            scratchpad_size,
            total_pads: total_pads_count,
            occupied_pads: occupied_pads_count,
            free_pads: free_pads_count,
            total_space_bytes,
            occupied_pad_space_bytes,
            free_pad_space_bytes,
            occupied_data_bytes: occupied_data_size_total,
            wasted_space_bytes,
        })
    }

    async fn save_master_index(&self) -> Result<(), Error> {
        crate::storage::storage_save_mis_from_arc_static(
            &self.storage.client(),
            &self.master_index_addr,
            &self.storage.get_master_index_info().1,
            &self.master_index_storage,
        )
        .await
    }

    pub async fn list_keys(&self) -> Result<Vec<String>, Error> {
        debug!("MutAnt: list_keys called");
        let guard = self.master_index_storage.lock().await;
        let keys = guard.index.keys().cloned().collect();
        Ok(keys)
    }

    /// Retrieves a list of keys along with their size and modification time.
    pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error> {
        debug!("MutAnt: list_key_details called");
        let guard = self.master_index_storage.lock().await;
        let details: Vec<KeyDetails> = guard
            .index
            .iter()
            .map(|(key, info)| KeyDetails {
                key: key.clone(),
                size: info.data_size,
                modified: info.modified,
            })
            .collect();
        Ok(details)
    }
}
