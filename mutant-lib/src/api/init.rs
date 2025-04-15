use crate::data::manager::{DataManager, DefaultDataManager};
use crate::error::Error; // Assuming top-level Error enum
use crate::events::{invoke_init_callback, InitCallback, InitProgressEvent};
use crate::index::manager::{DefaultIndexManager, IndexManager};
use crate::network::adapter::{AutonomiNetworkAdapter, NetworkAdapter};
use crate::pad_lifecycle::manager::{DefaultPadLifecycleManager, PadLifecycleManager};
use crate::storage::manager::{DefaultStorageManager, StorageManager};
use crate::types::MutAntConfig;
use autonomi::{ScratchpadAddress, SecretKey};
use hex;
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::sync::Arc;

// Define the number of initialization steps for progress reporting
const TOTAL_INIT_STEPS: u64 = 6; // 1:Wallet, 2:DeriveKey, 3:NetworkAdapter, 4:Managers, 5:IndexInit, 6:DataManager

/// Helper function to derive the master index key and address from the user's private key.
/// This logic is moved here from the old MutAnt::init.
fn derive_master_index_info(
    private_key_hex: &str,
) -> Result<(ScratchpadAddress, SecretKey), Error> {
    debug!("Deriving master index key and address...");
    let hex_to_decode = if private_key_hex.starts_with("0x") {
        &private_key_hex[2..]
    } else {
        private_key_hex
    };

    let input_key_bytes = hex::decode(hex_to_decode)
        .map_err(|e| Error::Config(format!("Failed to decode private key hex: {}", e)))?;

    let mut hasher = Sha256::new();
    hasher.update(&input_key_bytes);
    let hash_result = hasher.finalize();
    let key_array: [u8; 32] = hash_result.into();

    let derived_key = SecretKey::from_bytes(key_array)
        .map_err(|e| Error::Internal(format!("Failed to create SecretKey from HASH: {:?}", e)))?;
    let derived_public_key = derived_key.public_key();
    let address = ScratchpadAddress::new(derived_public_key);
    info!("Derived Master Index Address: {}", address);
    Ok((address, derived_key))
}

/// Initializes all layers and returns Arcs to the necessary managers.
/// This function encapsulates the complex initialization sequence.
#[allow(clippy::too_many_arguments)] // Necessary complexity for initialization
pub(crate) async fn initialize_layers(
    private_key_hex: &str,
    config: &MutAntConfig,
    mut init_callback: Option<InitCallback>,
) -> Result<
    (
        // Return tuple of Arcs needed by MutAnt struct
        Arc<dyn DataManager>,
        Arc<dyn PadLifecycleManager>,
        Arc<dyn IndexManager>,
        Arc<dyn NetworkAdapter>, // Pass NetworkAdapter too if MutAnt needs direct access (e.g., get_network_choice)
        ScratchpadAddress,       // Return master index address
        SecretKey,               // Return master index key
    ),
    Error, // Use the top-level Error
> {
    info!("Initializing MutAnt layers with config: {:?}", config);
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Starting {
            total_steps: TOTAL_INIT_STEPS,
        },
    )
    .await?;

    // --- Step 1 & 2: Derive Master Index Key/Address ---
    // Wallet creation is now part of NetworkAdapter::new
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 1,
            message: "Validating private key...".to_string(),
        },
    )
    .await?;
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 2,
            message: "Deriving master index key...".to_string(),
        },
    )
    .await?;
    let (master_index_address, master_index_key) = derive_master_index_info(private_key_hex)?;

    // --- Step 3: Instantiate Network Layer ---
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 3,
            message: "Initializing network connection...".to_string(),
        },
    )
    .await?;
    let network_adapter =
        Arc::new(AutonomiNetworkAdapter::new(private_key_hex, config.network).await?);
    info!("NetworkAdapter initialized.");

    // --- Step 4: Instantiate Lower Managers ---
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 4,
            message: "Initializing core managers...".to_string(),
        },
    )
    .await?;

    // Clone the Arc<ConcreteType> and then explicitly cast to Arc<dyn Trait>
    let storage_manager = Arc::new(DefaultStorageManager::new(
        Arc::clone(&network_adapter) as Arc<dyn NetworkAdapter>
    ));

    let index_manager = Arc::new(DefaultIndexManager::new(
        Arc::clone(&storage_manager) as Arc<dyn StorageManager>
    ));

    let pad_lifecycle_manager = Arc::new(DefaultPadLifecycleManager::new(
        Arc::clone(&index_manager) as Arc<dyn IndexManager>,
        Arc::clone(&network_adapter) as Arc<dyn NetworkAdapter>,
    ));
    info!("StorageManager, IndexManager, PadLifecycleManager initialized.");

    // --- Step 5: Initialize Index State (Cache -> Remote -> Prompt) ---
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 5,
            message: "Loading/Initializing master index...".to_string(),
        },
    )
    .await?;
    let prompt_needed =
        !pad_lifecycle_manager // `initialize_index` returns true if loaded, false if default/prompt needed
            .initialize_index(&master_index_address, &master_index_key, config.network)
            .await?;

    if prompt_needed {
        info!("Index not found remotely. Prompting user for creation...");
        let user_response = invoke_init_callback(
            &mut init_callback,
            InitProgressEvent::PromptCreateRemoteIndex,
        )
        .await?;

        match user_response {
            Some(true) => {
                info!("User confirmed remote index creation. Saving default index...");
                invoke_init_callback(
                    &mut init_callback,
                    InitProgressEvent::Step {
                        step: 5,
                        message: "Creating remote master index...".to_string(),
                    },
                )
                .await?; // Reuse step 5 for sub-step
                         // Save the default index (which is now in memory via IndexManager) remotely
                if let Err(e) = index_manager
                    .save(&master_index_address, &master_index_key)
                    .await
                {
                    error!("Failed to save newly created default index remotely: {}", e);
                    // Should we fail init here? Yes, if user wanted remote but it failed.
                    invoke_init_callback(
                        &mut init_callback,
                        InitProgressEvent::Failed {
                            error_msg: format!("Failed to create remote index: {}", e),
                        },
                    )
                    .await
                    .ok(); // Ignore result
                    return Err(e.into()); // Convert IndexError to top-level Error
                }
                info!("Default index saved remotely.");
                // Also ensure cache is updated (should have happened in initialize_index, but maybe save again?)
                if let Err(e) = pad_lifecycle_manager.save_index_cache(config.network).await {
                    warn!("Failed to save index cache after remote creation: {}", e);
                    // Don't fail init for cache write error
                }
            }
            Some(false) => {
                info!("User declined remote index creation. Proceeding with local index only.");
            }
            None => {
                // No callback provided or callback returned None/Err
                warn!("No user response for remote index creation prompt, or callback error. Aborting initialization.");
                invoke_init_callback(
                    &mut init_callback,
                    InitProgressEvent::Failed {
                        error_msg: "Aborted due to missing response for remote index creation."
                            .to_string(),
                    },
                )
                .await
                .ok();
                return Err(Error::Config(
                    "Initialization aborted: User interaction required for remote index creation."
                        .to_string(),
                ));
            }
        }
    }
    info!("Index state initialization complete.");

    // --- Step 6: Instantiate Data Manager ---
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 6,
            message: "Initializing data manager...".to_string(),
        },
    )
    .await?;
    let data_manager = Arc::new(DefaultDataManager::new(
        Arc::clone(&index_manager) as Arc<dyn IndexManager>,
        Arc::clone(&pad_lifecycle_manager) as Arc<dyn PadLifecycleManager>,
        Arc::clone(&storage_manager) as Arc<dyn StorageManager>,
    ));
    info!("DataManager initialized.");

    // --- Complete ---
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Complete {
            message: "Initialization complete.".to_string(),
        },
    )
    .await?;
    info!("All layers initialized successfully.");

    Ok((
        data_manager,
        pad_lifecycle_manager,
        index_manager,
        network_adapter,
        master_index_address,
        master_index_key,
    ))
}
