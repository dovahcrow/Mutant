use crate::error::Error;
use crate::events::{GetCallback, InitCallback, PutCallback};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::pad_manager::PadManager;
use crate::storage::Storage;
use autonomi::{Network, ScratchpadAddress, SecretKey, Wallet};
use chrono::{DateTime, Utc};
use hex;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub mod data_structures;
pub mod remove_logic;
pub mod store_logic;
pub mod update_logic;

// --- Network Configuration ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkChoice {
    Devnet,
    Mainnet,
}

// --- MutAnt Configuration ---
#[derive(Debug, Clone)]
pub struct MutAntConfig {
    pub network: NetworkChoice,
}
impl MutAntConfig {
    pub fn local() -> Self {
        Self {
            network: NetworkChoice::Devnet,
        }
    }

    pub fn mainnet() -> Self {
        Self {
            network: NetworkChoice::Mainnet,
        }
    }
}

impl Default for MutAntConfig {
    fn default() -> Self {
        Self {
            network: NetworkChoice::Mainnet,
        }
    }
}

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
    /// Creates a new MutAnt instance using default configuration.
    /// Use `init_with_progress` for detailed initialization feedback and custom config.
    pub async fn init(private_key_hex: String) -> Result<(Self, Option<JoinHandle<()>>), Error> {
        Self::init_with_progress(private_key_hex, MutAntConfig::default(), None).await
    }

    /// Creates a new MutAnt instance with optional progress reporting via callback and custom config.
    pub async fn init_with_progress(
        private_key_hex: String,
        config: MutAntConfig,
        mut init_callback: Option<InitCallback>,
    ) -> Result<(Self, Option<JoinHandle<()>>), Error> {
        info!("Initializing MutAnt with config: {:?}...", config);

        let use_local = match config.network {
            NetworkChoice::Devnet => true,
            NetworkChoice::Mainnet => false,
        };

        let network = match Network::new(use_local) {
            Ok(n) => n,
            Err(e) => {
                error!("Failed to initialize Autonomi network: {}", e);
                return Err(Error::NetworkInitError(format!(
                    "Failed to initialize Autonomi network: {}",
                    e
                )));
            }
        };

        info!("Autonomi network initialized.");

        let wallet = match Wallet::new_from_private_key(network.clone(), &private_key_hex) {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to create wallet from private key: {}", e);
                return Err(Error::WalletError(format!(
                    "Failed to create wallet from private key: {}",
                    e
                )));
            }
        };

        info!("Autonomi wallet created.");

        {
            info!("Initializing Storage layer...");

            let (storage_instance, storage_init_handle, mis_mutex): (
                Storage,
                Option<JoinHandle<()>>,
                Arc<Mutex<MasterIndexStorage>>,
            ) = match crate::storage::new(
                wallet,
                private_key_hex.clone(),
                config.network,
                &mut init_callback,
            )
            .await
            {
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
    pub async fn store(&self, user_key: &str, data_bytes: &[u8]) -> Result<(), Error> {
        self.store_with_progress(user_key, data_bytes, None).await
    }

    /// Stores raw bytes under a given user key with optional progress reporting via callback.
    pub async fn store_with_progress(
        &self,
        user_key: &str,
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
    pub async fn update(&self, user_key: &str, data_bytes: &[u8]) -> Result<(), Error> {
        self.update_with_progress(user_key, data_bytes, None).await
    }

    /// Updates the raw bytes associated with an existing user key, with optional progress reporting.
    ///
    /// Returns `Error::KeyNotFound` if the key does not exist.
    pub async fn update_with_progress(
        &self,
        user_key: &str,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!("MutAnt::update: Entered for key '{}'", user_key);
        if user_key == MASTER_INDEX_KEY {
            error!("Attempted to update the master index key directly. Denying access.");
            return Err(Error::OperationNotSupported);
        }
        let result = update_logic::update_item(self, user_key, data_bytes, callback).await;
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

    /// Saves the current in-memory master index to the remote backend.
    pub async fn save_master_index(&self) -> Result<(), Error> {
        crate::storage::storage_save_mis_from_arc_static(
            &self.storage.client(),
            &self.master_index_addr,
            &self.storage.get_master_index_info().1,
            &self.master_index_storage,
        )
        .await
    }

    /// Imports a free scratchpad using its private key hex string.
    /// The pad must not already be tracked (either as free or occupied).
    pub async fn import_free_pad(&self, private_key_hex: &str) -> Result<(), Error> {
        info!(
            "Importing free pad with key starting: {}...",
            &private_key_hex[..8]
        );

        // 1. Parse the private key
        let key_bytes = hex::decode(private_key_hex)
            .map_err(|e| Error::InvalidInput(format!("Invalid private key hex: {}", e)))?;
        // Ensure the key has the correct length (32 bytes)
        let key_array: [u8; 32] = key_bytes.as_slice().try_into().map_err(|_| {
            Error::InvalidInput(format!(
                "Private key has incorrect length. Expected 32 bytes, got {}",
                key_bytes.len()
            ))
        })?;
        let secret_key = SecretKey::from_bytes(key_array) // Use the array directly, not a reference
            .map_err(|e| Error::InvalidInput(format!("Invalid private key: {}", e)))?;
        let public_key = secret_key.public_key();
        let pad_address = ScratchpadAddress::new(public_key);

        info!("Derived address for import: {}", pad_address);

        // 2. Lock Master Index and check for conflicts
        let mut mis_guard = self.master_index_storage.lock().await;
        debug!("ImportPad[{}]: Master index lock acquired.", pad_address);

        // Check if already in free pads
        if mis_guard
            .free_pads
            .iter()
            .any(|(addr, _)| *addr == pad_address)
        {
            debug!(
                "ImportPad[{}]: Pad already exists in free list. Aborting.",
                pad_address
            );
            return Err(Error::PadAlreadyExists(format!(
                "Pad {} is already in the free list.",
                pad_address
            )));
        }

        // Check if used by any key
        if mis_guard
            .index
            .values()
            .any(|key_info| key_info.pads.iter().any(|(addr, _)| *addr == pad_address))
        {
            debug!(
                "ImportPad[{}]: Pad is currently occupied by a key. Aborting.",
                pad_address
            );
            return Err(Error::PadAlreadyExists(format!(
                "Pad {} is currently occupied by a key.",
                pad_address
            )));
        }

        // 3. Add to free pads list
        info!("ImportPad[{}]: Adding pad to free list.", pad_address);
        mis_guard.free_pads.push((pad_address, key_bytes));

        // 4. Save Master Index
        // Drop the lock before saving to avoid holding it during potentially long I/O
        drop(mis_guard);
        debug!("ImportPad[{}]: Master index lock released.", pad_address);

        info!("ImportPad[{}]: Saving updated master index...", pad_address);
        self.save_master_index().await?;
        info!(
            "ImportPad[{}]: Successfully imported and saved master index.",
            pad_address
        );

        Ok(())
    }

    /// Resets the master index to its initial empty state.
    /// WARNING: This is a destructive operation and will orphan existing data pads.
    pub async fn reset_master_index(&self) -> Result<(), Error> {
        info!("Resetting master index requested.");

        // 1. Lock the master index storage
        let mut mis_guard = self.master_index_storage.lock().await;
        info!("Acquired lock on master index storage.");

        // 2. Reset the in-memory state to default, but set the correct scratchpad size
        *mis_guard = MasterIndexStorage {
            scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
            ..Default::default() // Use default for all other fields
        };
        info!(
            "In-memory master index reset. Size set to: {}",
            DEFAULT_SCRATCHPAD_SIZE
        );

        // 3. Drop the guard before saving to avoid deadlock in save_master_index
        drop(mis_guard);
        info!("Released lock on master index storage.");

        // 4. Persist the empty/default master index
        info!("Attempting to save the reset master index...");
        match self.save_master_index().await {
            Ok(_) => {
                info!("Successfully saved the reset master index.");
                Ok(())
            }
            Err(e) => {
                error!("Failed to save the reset master index: {}", e);
                Err(e)
            }
        }
    }

    /// Lists all user keys currently tracked in the master index.
    pub async fn list_keys(&self) -> Vec<String> {
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

    /// Internal method to update the in-memory master index storage.
    /// Used primarily by the sync command.
    pub async fn update_internal_master_index(
        &self,
        new_index: MasterIndexStorage,
    ) -> Result<(), Error> {
        debug!("MutAnt: Updating internal master index storage directly.");
        let mut guard = self.master_index_storage.lock().await;
        *guard = new_index;
        Ok(())
    }

    /// Fetches the MasterIndexStorage directly from the remote backend, bypassing the local cache.
    pub async fn fetch_remote_master_index(&self) -> Result<MasterIndexStorage, Error> {
        debug!("MutAnt: Fetching remote master index directly...");
        let (address, key) = self.storage.get_master_index_info();
        crate::storage::fetch_remote_master_index_storage_static(
            self.storage.client(),
            &address,
            &key,
        )
        .await // Return the result directly
    }
}
