use crate::data::manager::DefaultDataManager;
use crate::index::manager::DefaultIndexManager;
use crate::internal_error::Error;
use crate::internal_events::{invoke_init_callback, InitCallback, InitProgressEvent};
use crate::network::AutonomiNetworkAdapter;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use crate::types::MutAntConfig;
use autonomi::{ScratchpadAddress, SecretKey};
use hex;
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const TOTAL_INIT_STEPS: u64 = 6;

fn derive_master_index_info(
    private_key_hex: &str,
) -> Result<(ScratchpadAddress, SecretKey), Error> {
    debug!("Deriving master index key and address...");
    let hex_to_decode = private_key_hex
        .strip_prefix("0x")
        .unwrap_or(private_key_hex);

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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn initialize_layers(
    private_key_hex: &str,
    config: &MutAntConfig,
    mut init_callback: Option<InitCallback>,
) -> Result<
    (
        Arc<DefaultDataManager>,
        Arc<DefaultPadLifecycleManager>,
        Arc<DefaultIndexManager>,
        Arc<AutonomiNetworkAdapter>,
        ScratchpadAddress,
        SecretKey,
    ),
    Error,
> {
    info!("Initializing MutAnt layers with config: {:?}", config);
    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Starting {
            total_steps: TOTAL_INIT_STEPS,
        },
    )
    .await?;

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

    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 3,
            message: "Initializing network connection...".to_string(),
        },
    )
    .await?;

    let network_adapter_concrete = AutonomiNetworkAdapter::new(private_key_hex, config.network)?;

    let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(network_adapter_concrete);
    info!("NetworkAdapter configuration initialized.");

    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 4,
            message: "Initializing core managers...".to_string(),
        },
    )
    .await?;

    let index_manager = Arc::new(DefaultIndexManager::new(
        Arc::clone(&network_adapter),
        master_index_key.clone(),
    ));

    let pad_lifecycle_manager = Arc::new(DefaultPadLifecycleManager::new(
        Arc::clone(&index_manager),
        Arc::clone(&network_adapter),
    ));
    info!("IndexManager, PadLifecycleManager initialized.");

    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 5,
            message: "Loading/Initializing master index...".to_string(),
        },
    )
    .await?;

    let prompt_needed = !pad_lifecycle_manager
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
                .await?;

                if let Err(e) = index_manager
                    .save(&master_index_address, &master_index_key)
                    .await
                {
                    error!("Failed to save newly created default index remotely: {}", e);

                    invoke_init_callback(
                        &mut init_callback,
                        InitProgressEvent::Failed {
                            error_msg: format!("Failed to create remote index: {}", e),
                        },
                    )
                    .await
                    .ok();
                    return Err(e.into());
                }
                info!("Default index saved remotely.");

                if let Err(e) = pad_lifecycle_manager.save_index_cache(config.network).await {
                    warn!("Failed to save index cache after remote creation: {}", e);
                }
            }
            Some(false) => {
                info!("User declined remote index creation. Proceeding with local index only.");
            }
            None => {
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

    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Step {
            step: 6,
            message: "Initializing data manager...".to_string(),
        },
    )
    .await?;
    let data_manager = Arc::new(DefaultDataManager::new(
        Arc::clone(&network_adapter),
        Arc::clone(&index_manager),
        Arc::clone(&pad_lifecycle_manager),
    ));
    info!("DataManager initialized.");

    invoke_init_callback(
        &mut init_callback,
        InitProgressEvent::Complete {
            message: "Initialization complete.".to_string(),
        },
    )
    .await
    .map_err(|e| Error::Internal(format!("Callback invocation failed: {}", e)))?;
    info!("MutAnt layers initialization completed successfully.");

    Ok((
        data_manager,
        pad_lifecycle_manager,
        index_manager,
        network_adapter,
        master_index_address,
        master_index_key,
    ))
}
