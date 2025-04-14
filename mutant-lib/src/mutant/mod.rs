use crate::cache::{read_local_index, write_local_index};
use crate::error::Error;
use crate::events::{
    invoke_init_callback, invoke_purge_callback, GetCallback, InitCallback, PurgeCallback,
    PurgeEvent, PutCallback,
};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::pad_manager::PadManager;
use crate::storage::{storage_create_mis_from_arc_static, Storage};
use crate::InitProgressEvent;
use autonomi::{Network, ScratchpadAddress, SecretKey, Wallet};
use chrono::{DateTime, Utc};
use hex;
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc; // For signaling completion from tasks
use tokio::sync::Mutex;
use tokio::task::JoinSet;

// Bring the autonomi crate into scope to potentially resolve trait methods
use crate::autonomi;

pub mod data_structures;
pub mod remove_logic;
pub mod store_logic;
pub mod update_logic;

const TOTAL_INIT_STEPS: u32 = 6; // Define the constant (Added step for remote MIS creation)

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
    /// The total number of scratchpads managed (occupied + free + pending).
    pub total_pads: usize,
    /// The number of scratchpads currently holding data.
    pub occupied_pads: usize,
    /// The number of scratchpads available for reuse.
    pub free_pads: usize,
    /// The number of scratchpads pending verification.
    pub pending_verification_pads: usize,
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
#[derive(Debug, Clone, PartialEq)]
pub struct KeyDetails {
    pub key: String,
    pub size: usize,
    pub modified: DateTime<Utc>,
    pub is_finished: bool,
    pub completion_percentage: Option<f32>,
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
    pub async fn init(private_key_hex: String) -> Result<Self, Error> {
        Self::init_with_progress(private_key_hex, MutAntConfig::default(), None).await
    }

    /// Creates a new MutAnt instance with optional progress reporting via callback and custom config.
    pub async fn init_with_progress(
        private_key_hex: String,
        config: MutAntConfig,
        mut init_callback: Option<InitCallback>,
    ) -> Result<Self, Error> {
        info!("Initializing MutAnt with config: {:?}...", config);

        #[allow(unused_assignments)]
        let mut current_step = 0;

        macro_rules! try_step {
            ($step:expr, $message:expr, $action:expr) => {{
                current_step = $step;
                invoke_init_callback(
                    &mut init_callback,
                    // Use the correct Step variant signature
                    InitProgressEvent::Step {
                        step: current_step as u64, // Convert to u64 if needed
                        message: $message.to_string(),
                    },
                )
                .await?;
                match $action.await {
                    Ok(val) => val,
                    Err(e) => {
                        invoke_init_callback(
                            &mut init_callback,
                            InitProgressEvent::Failed {
                                error_msg: e.to_string(),
                            },
                        )
                        .await
                        .ok(); // Ignore error during failure reporting
                        return Err(e);
                    }
                }
            }};
        }

        invoke_init_callback(
            &mut init_callback,
            InitProgressEvent::Starting {
                total_steps: TOTAL_INIT_STEPS as u64,
            }, // Convert to u64
        )
        .await?;

        // --- Step 1: Create Wallet ---
        let wallet = try_step!(1, "Creating wallet", async {
            // Revert to using Network and new_from_private_key
            let network = Network::new(config.network == NetworkChoice::Devnet)
                .map_err(|e| Error::NetworkInitError(format!("Network init failed: {}", e)))?;
            Wallet::new_from_private_key(network, &private_key_hex)
                .map_err(|e| Error::WalletError(format!("Failed to create wallet: {}", e)))
        });
        info!("Autonomi wallet created.");

        // --- Step 2: Derive Master Index Key and Address --- (Local CPU only)
        let (master_index_address, master_index_key) =
            try_step!(2, "Deriving storage key and address", async {
                let hex_to_decode = if private_key_hex.starts_with("0x") {
                    &private_key_hex[2..]
                } else {
                    &private_key_hex
                };
                let input_key_bytes = hex::decode(hex_to_decode).map_err(|e| {
                    Error::InvalidInput(format!("Failed to decode private key hex: {}", e))
                })?;
                let mut hasher = Sha256::new();
                hasher.update(&input_key_bytes);
                let hash_result = hasher.finalize();
                let key_array: [u8; 32] = hash_result.into();
                let derived_key = SecretKey::from_bytes(key_array).map_err(|e| {
                    Error::InternalError(format!("Failed to create SecretKey from HASH: {:?}", e))
                })?;
                let derived_public_key = derived_key.public_key();
                let address = ScratchpadAddress::new(derived_public_key);
                info!("Derived Master Index Address: {}", address);
                Ok::<_, Error>((address, derived_key))
            });

        // --- Step 3: Initialize Storage Layer (Lazy) --- (No network yet)
        let storage_instance = try_step!(3, "Initializing storage layer (lazy)", async {
            crate::storage::new(
                wallet.clone(), // Pass the created wallet
                config.network,
                master_index_address,
                master_index_key.clone(), // Clone key for storage
            )
        });
        let storage_arc = Arc::new(storage_instance);
        info!("Storage layer initialized (lazily). Network client init deferred.");

        // --- Step 4: Load or Create Master Index --- (Cache -> Network -> Create)
        let mis_mutex = {
            current_step = 4;
            invoke_init_callback(
                &mut init_callback,
                InitProgressEvent::Step {
                    step: current_step as u64,
                    message: "Loading master index (cache)".to_string(),
                },
            )
            .await?;

            let network_choice = config.network;
            match read_local_index(network_choice).await {
                Ok(Some(mut cached_index)) => {
                    info!("Loaded index from local cache.");
                    // Ensure scratchpad size is set
                    if cached_index.scratchpad_size == 0 {
                        warn!(
                            "Cached index has scratchpad_size 0, setting to default: {}",
                            DEFAULT_SCRATCHPAD_SIZE
                        );
                        cached_index.scratchpad_size = DEFAULT_SCRATCHPAD_SIZE;
                        if let Err(e) = write_local_index(&cached_index, network_choice).await {
                            warn!(
                                "Failed to write updated cache after fixing scratchpad_size: {}",
                                e
                            );
                        }
                    }
                    Ok::<_, Error>(Arc::new(Mutex::new(cached_index)))
                }
                Ok(None) => {
                    info!(
                        "Local cache not found. Attempting to load from network or create new..."
                    );
                    invoke_init_callback(
                        &mut init_callback,
                        InitProgressEvent::Step {
                            step: current_step as u64, // Still step 4
                            message: "Checking remote master index...".to_string(),
                        },
                    )
                    .await?;

                    // Use the storage instance method to load/create from network
                    match storage_arc.load_or_create_master_index().await {
                        Ok(network_mis_arc) => {
                            info!("Successfully loaded index from network.");
                            // Save the newly loaded index back to local cache
                            let mis_guard = network_mis_arc.lock().await;
                            if let Err(e) = write_local_index(&*mis_guard, network_choice).await {
                                warn!("Failed to write network-loaded index to local cache: {}", e);
                            }
                            drop(mis_guard);
                            Ok(network_mis_arc)
                        }
                        Err(Error::MasterIndexNotFound) => {
                            // Network load explicitly failed because index doesn't exist.
                            // Prompt the user via callback.
                            info!("Master index not found on network. Prompting user...");
                            invoke_init_callback(
                                &mut init_callback,
                                InitProgressEvent::PromptCreateRemoteIndex,
                            )
                            .await?;

                            // Inform the user that creation is starting
                            // --- Step 5: Create Remote Index ---
                            invoke_init_callback(
                                &mut init_callback,
                                InitProgressEvent::Step {
                                    step: 5, // Explicitly set step 5 for this event
                                    message: "Creating remote master index...".to_string(),
                                },
                            )
                            .await?;

                            // Callback result was already handled by the invoke_init_callback
                            // that emitted PromptCreateRemoteIndex. If we got here, user confirmed.

                            warn!("No remote index exists. Creating and saving a new default index locally and remotely.");
                            let default_mis = MasterIndexStorage {
                                scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                                ..Default::default()
                            };
                            let mis_arc = Arc::new(Mutex::new(default_mis));

                            // Attempt to save the new default index remotely
                            let (master_addr, master_key) = storage_arc.get_master_index_info();
                            let client = storage_arc.get_client().await?; // Ensure client is initialized
                            let wallet_instance = storage_arc.wallet(); // Use the getter
                            match storage_create_mis_from_arc_static(
                                client,
                                wallet_instance, // Pass the wallet reference
                                &master_addr,
                                &master_key,
                                &mis_arc,
                            )
                            .await
                            {
                                Ok(_) => {
                                    info!(
                                        "Successfully created new default master index remotely."
                                    );
                                    // Also save locally
                                    let mis_guard = mis_arc.lock().await;
                                    if let Err(e) =
                                        write_local_index(&*mis_guard, network_choice).await
                                    {
                                        warn!("Failed to write newly created index to local cache: {}", e);
                                    }
                                    drop(mis_guard);
                                }
                                Err(e) => {
                                    // If creating remotely fails, still proceed with the in-memory index
                                    // but log the error. The cache won't be written in this case.
                                    error!("Failed to create default index on remote: {}. Proceeding with in-memory index only.", e);
                                }
                            }
                            Ok(mis_arc)
                        }
                        Err(e) => {
                            // Other errors during network load (e.g., Cbor, connection issues)
                            error!("Failed to load index from network: {}. Initializing empty default index in memory only.", e);
                            let default_mis = MasterIndexStorage {
                                scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                                ..Default::default()
                            };
                            Ok(Arc::new(Mutex::new(default_mis)))
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to read local cache: {}. Initializing empty default index in memory.", e);
                    let default_mis = MasterIndexStorage {
                        scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                        ..Default::default()
                    };
                    Ok(Arc::new(Mutex::new(default_mis)))
                }
            }
        };

        // Now handle the result of the whole block above
        let mis_mutex = match mis_mutex {
            Ok(val) => val,
            Err(e) => {
                invoke_init_callback(
                    &mut init_callback,
                    InitProgressEvent::Failed {
                        error_msg: e.to_string(),
                    },
                )
                .await
                .ok(); // Ignore error during failure reporting
                return Err(e);
            }
        };

        // --- Step 5: Initialize PadManager --- (No network)
        let pad_manager = try_step!(6, "Initializing pad manager", async {
            Ok::<_, Error>(PadManager::new(storage_arc.clone(), mis_mutex.clone()))
        });
        info!("PadManager initialized.");

        let mutant = Self {
            storage: storage_arc,
            pad_manager,
            master_index_storage: mis_mutex,
            master_index_addr: master_index_address, // Store the derived address
        };

        info!("MutAnt initialization complete.");
        invoke_init_callback(
            &mut init_callback,
            InitProgressEvent::Complete {
                message: "Initialization complete.".to_string(),
            },
        )
        .await?;
        Ok(mutant)
    }

    /// Stores raw bytes under a given user key without progress reporting.
    /// Use `store_with_progress` for detailed feedback.
    pub async fn store(&self, user_key: &str, data_bytes: &[u8]) -> Result<(), Error> {
        // Create a dummy counter Arc for the call, as it won't be observed
        let commit_counter_arc = Arc::new(tokio::sync::Mutex::new(0u64));
        self.store_with_progress(user_key, data_bytes, None, commit_counter_arc)
            .await
    }

    /// Stores raw bytes under a given user key with optional progress reporting via callback.
    pub async fn store_with_progress(
        &self,
        user_key: &str,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
        commit_counter_arc: Arc<tokio::sync::Mutex<u64>>,
    ) -> Result<(), Error> {
        if user_key == MASTER_INDEX_KEY {
            return Err(Error::InvalidInput(
                "User key conflicts with internal master index key".to_string(),
            ));
        }
        // Pass the counter Arc down to store_data
        store_logic::store_data(self, user_key, data_bytes, callback, commit_counter_arc).await
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
        let pending_verification_pads_count = mis_guard.pending_verification_pads.len();
        let mut occupied_pads_count = 0;
        let mut occupied_data_size_total: u64 = 0;

        for key_info in mis_guard.index.values() {
            occupied_pads_count += key_info.pads.len();
            occupied_data_size_total += key_info.data_size as u64;
        }

        let total_pads_count =
            occupied_pads_count + free_pads_count + pending_verification_pads_count;

        let scratchpad_size_u64 = scratchpad_size as u64;
        let occupied_pad_space_bytes = occupied_pads_count as u64 * scratchpad_size_u64;
        let free_pad_space_bytes = free_pads_count as u64 * scratchpad_size_u64;
        let total_space_bytes = total_pads_count as u64 * scratchpad_size_u64;

        let wasted_space_bytes = occupied_pad_space_bytes.saturating_sub(occupied_data_size_total);

        debug!(
            "Stats calculated: total={}, occupied={}, free={}, pending={}, total_space={}, data_size={}",
            total_pads_count, occupied_pads_count, free_pads_count, pending_verification_pads_count, total_space_bytes, occupied_data_size_total
        );
        debug!("Master index lock released after stats.");

        Ok(StorageStats {
            scratchpad_size,
            total_pads: total_pads_count,
            occupied_pads: occupied_pads_count,
            free_pads: free_pads_count,
            pending_verification_pads: pending_verification_pads_count,
            total_space_bytes,
            occupied_pad_space_bytes,
            free_pad_space_bytes,
            occupied_data_bytes: occupied_data_size_total,
            wasted_space_bytes,
        })
    }

    /// Saves the current in-memory master index to the remote backend.
    pub async fn save_master_index(&self) -> Result<(), Error> {
        debug!("MutAnt: Saving master index remotely...");
        // --- LAZY CLIENT GET & CALL ---
        let client = self.storage.get_client().await?;
        let (address, key) = self.storage.get_master_index_info();
        crate::storage::storage_save_mis_from_arc_static(
            client, // Pass the obtained client
            &address,
            &key,
            &self.master_index_storage,
        )
        .await
        // --- END ---
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
        if mis_guard.index.values().any(|key_info| {
            key_info
                .pads
                .iter()
                .any(|pad_info| pad_info.address == pad_address)
        }) {
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

    /// Lists all user keys currently tracked, along with completion status and percentage (if incomplete).
    pub async fn list_keys(&self) -> Result<Vec<(String, bool, Option<f32>)>, Error> {
        let mis_guard = self.master_index_storage.lock().await;
        let keys_with_status: Vec<(String, bool, Option<f32>)> = mis_guard
            .index
            .iter()
            .filter(|(k, _)| *k != MASTER_INDEX_KEY)
            .map(|(key, info)| {
                let is_complete = info.is_complete;
                let percentage = if !is_complete && !info.pads.is_empty() {
                    Some((info.populated_pads_count as f32 / info.pads.len() as f32) * 100.0)
                } else {
                    None
                };
                (key.clone(), is_complete, percentage)
            })
            .collect();
        drop(mis_guard);
        Ok(keys_with_status)
    }

    /// Retrieves a list of keys along with their size, modification time, completion status, and percentage.
    pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error> {
        debug!("MutAnt: list_key_details called");
        let guard = self.master_index_storage.lock().await;
        let details: Vec<KeyDetails> = guard
            .index
            .iter()
            .filter(|(k, _)| *k != MASTER_INDEX_KEY) // Exclude internal keys
            .map(|(key, info)| {
                let is_complete = info.is_complete;
                let percentage = if !is_complete && !info.pads.is_empty() {
                    Some((info.populated_pads_count as f32 / info.pads.len() as f32) * 100.0)
                } else {
                    None
                };
                KeyDetails {
                    key: key.clone(),
                    size: info.data_size,
                    modified: info.modified,
                    is_finished: is_complete,
                    completion_percentage: percentage,
                }
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
        // --- LAZY CLIENT GET & CALL ---
        let client = self.storage.get_client().await?;
        let (address, key) = self.storage.get_master_index_info();
        crate::storage::fetch_remote_master_index_storage_static(
            client, // Pass the obtained client
            &address, &key,
        )
        .await
        // --- END ---
    }

    /// Purges pads listed in `pending_verification_pads` by checking their existence concurrently.
    /// Attempts to fetch metadata for each pad. Successfully fetched pads are moved to `free_pads`.
    /// Failed pads (e.g., not found) are discarded.
    /// Saves the updated master index ONLY to the local cache.
    /// Uses an optional callback for progress reporting.
    pub async fn purge_unverified_pads(
        &self,
        mut callback: Option<PurgeCallback>,
    ) -> Result<(), Error> {
        info!("Starting concurrent local purge of unverified pads...");

        let network_choice = self.storage.get_network_choice();
        let mut master_index_storage = self.master_index_storage.lock().await;

        let pads_to_verify = std::mem::take(&mut master_index_storage.pending_verification_pads);
        let initial_count = pads_to_verify.len();
        let verified_pads_arc = Arc::new(Mutex::new(Vec::new())); // Collect verified pads concurrently
        let failed_pads_count_arc = Arc::new(AtomicUsize::new(0)); // Count failures concurrently

        // Emit Starting event
        invoke_purge_callback(
            &mut callback,
            PurgeEvent::Starting {
                total_count: initial_count,
            },
        )
        .await?;

        info!(
            "Found {} pads pending verification. Starting concurrent fetch attempts...",
            initial_count
        );

        let mut first_error: Option<Error> = None;
        let mut callback_cancelled = false;

        if initial_count > 0 {
            let client = self.storage.get_client().await?;
            let client_arc = Arc::new(client.clone()); // Clone client for tasks
            let mut join_set = JoinSet::new();

            for (address, key) in pads_to_verify {
                let client_clone = Arc::clone(&client_arc);
                let verified_pads_clone = Arc::clone(&verified_pads_arc);
                let failed_count_clone = Arc::clone(&failed_pads_count_arc);

                join_set.spawn(async move {
                    debug!("Verifying pad task starting for address: {}", address);
                    match client_clone.scratchpad_get(&address).await {
                        Ok(_) => {
                            info!("Successfully verified pad at address: {}", address);
                            let mut verified_guard = verified_pads_clone.lock().await;
                            verified_guard.push((address.clone(), key.clone()));
                            // Signal success (no specific data needed, just the outcome)
                            Ok(true) // Indicate success
                        }
                        Err(e) => {
                            if e.to_string().contains("RecordNotFound") {
                                info!(
                                    "Pad at address {} not found. Discarding. Error: {}",
                                    address, e
                                );
                            } else {
                                warn!(
                                    "Failed to get pad at address {}: {}. Discarding.",
                                    address, e
                                );
                            }
                            failed_count_clone.fetch_add(1, Ordering::SeqCst);
                            // Signal failure, potentially returning the error if needed later
                            Err(e) // Indicate failure with error
                        }
                    }
                });
            }

            // Collect results and emit PadProcessed
            let mut processed_count = 0;
            while let Some(res) = join_set.join_next().await {
                processed_count += 1;
                debug!("Processed task {}/{}", processed_count, initial_count);
                match res {
                    Ok(task_result) => {
                        if let Err(task_err) = task_result {
                            if first_error.is_none() {
                                // Convert ScratchpadError to mutant_lib::Error
                                first_error = Some(Error::AutonomiClient(task_err));
                            }
                        }
                    }
                    Err(join_error) => {
                        // Task panicked or was cancelled
                        error!("JoinError during pad verification: {}", join_error);
                        failed_pads_count_arc.fetch_add(1, Ordering::SeqCst); // Count join errors as failures
                        if first_error.is_none() {
                            first_error = Some(Error::from_join_error_msg(
                                &join_error,
                                "Pad verification task failed".to_string(),
                            ));
                        }
                    }
                }
                // Emit PadProcessed event after each task finishes (success or failure)
                if !callback_cancelled {
                    if !invoke_purge_callback(&mut callback, PurgeEvent::PadProcessed).await? {
                        callback_cancelled = true;
                        if first_error.is_none() {
                            first_error = Some(Error::OperationCancelled);
                        }
                        join_set.abort_all(); // Cancel remaining tasks if callback requests it
                    }
                }
            }
        }

        let final_failed_count = failed_pads_count_arc.load(Ordering::SeqCst);
        let verified_pads = Arc::try_unwrap(verified_pads_arc)
            .map_err(|_| Error::InternalError("Failed to unwrap verified pads Arc".to_string()))?
            .into_inner();
        let final_verified_count = verified_pads.len();

        master_index_storage.free_pads.extend(verified_pads);

        info!(
            "Purge check complete. Verified: {}, Discarded: {}",
            final_verified_count, final_failed_count
        );

        // Save ONLY to local cache
        if initial_count > 0 {
            // Only write if changes were potentially made
            debug!(
                "Writing updated index to local cache ({:?})...",
                network_choice
            );
            // Clone the data to write *after* releasing the lock
            let data_to_write = master_index_storage.clone();
            drop(master_index_storage); // Release lock before write
            write_local_index(&data_to_write, network_choice).await?;
            info!("Local cache updated after purge.");
        } else {
            drop(master_index_storage); // Release lock even if no write
            info!("No pending pads found, local cache unchanged.");
        }

        // Emit Complete event
        invoke_purge_callback(
            &mut callback,
            PurgeEvent::Complete {
                verified_count: final_verified_count,
                failed_count: final_failed_count,
            },
        )
        .await?;

        // Return the first error encountered, if any
        if let Some(e) = first_error {
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Retrieves the network choice (Devnet or Mainnet) this MutAnt instance is configured for.
    pub fn get_network_choice(&self) -> NetworkChoice {
        self.storage.get_network_choice()
    }
}
