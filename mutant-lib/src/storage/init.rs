use super::network::{
    create_scratchpad_static, load_master_index_storage_static, storage_save_mis_from_arc_static,
};
use super::{ContentType, Storage};
use crate::error::Error;
use crate::events::{invoke_init_callback, InitCallback, InitProgressEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::mutant::DEFAULT_SCRATCHPAD_SIZE;
use autonomi::{client::payment::PaymentOption, Client, ScratchpadAddress, SecretKey, Wallet};
use hex;
use log::{debug, error, info, warn};
use serde_cbor;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub async fn new(
    wallet: Wallet,
    private_key_hex: String,
    init_callback: &mut Option<InitCallback>,
) -> Result<
    (
        Storage,
        Option<JoinHandle<()>>,
        Arc<Mutex<MasterIndexStorage>>,
    ),
    Error,
> {
    info!("Storage::new: Initializing storage layer...");

    let total_steps = 7;
    #[allow(unused_assignments)]
    let mut current_step = 0;

    macro_rules! try_step {
        ($step:expr, $message:expr, $action:expr) => {{
            current_step = $step;
            invoke_init_callback(
                init_callback,
                InitProgressEvent::Step {
                    step: current_step,
                    message: $message.to_string(),
                },
            )
            .await?;
            match $action.await {
                Ok(val) => val,
                Err(e) => {
                    invoke_init_callback(
                        init_callback,
                        InitProgressEvent::Failed {
                            error_msg: e.to_string(),
                        },
                    )
                    .await
                    .ok();
                    return Err(e);
                }
            }
        }};
    }

    invoke_init_callback(init_callback, InitProgressEvent::Starting { total_steps }).await?;

    let client = try_step!(1, "Initializing network client", async {
        Client::init_local()
            .await
            .map_err(|e| Error::NetworkConnectionFailed(e.to_string()))
    });

    let derived_key = try_step!(2, "Deriving storage key", async {
        let trimmed_hex = private_key_hex.trim();
        let hex_to_decode = if trimmed_hex.starts_with("0x") {
            &trimmed_hex[2..]
        } else {
            trimmed_hex
        };
        debug!(
            "Decoding hex for deterministic key HASH derivation: '{}'",
            hex_to_decode
        );
        let input_key_bytes = hex::decode(hex_to_decode)
            .map_err(|e| Error::InvalidInput(format!("Failed to decode private key hex: {}", e)))?;
        let mut hasher = Sha256::new();
        hasher.update(&input_key_bytes);
        let hash_result = hasher.finalize();
        debug!("Hashed input key bytes. Result: {:x}", hash_result);
        let key_array: [u8; 32] = hash_result.into();
        SecretKey::from_bytes(key_array).map_err(|e| {
            Error::InternalError(format!(
                "Failed to create SecretKey from HASH bytes: {:?}",
                e
            ))
        })
    });
    info!("Deterministically derived key from HASH of input hex.");

    current_step = 3;
    invoke_init_callback(
        init_callback,
        InitProgressEvent::Step {
            step: current_step,
            message: "Determining storage address".to_string(),
        },
    )
    .await?;
    let master_index_address = ScratchpadAddress::new(derived_key.public_key().into());
    info!(
        "Storage::new: Determined Master Index Address from DERIVED key: {}",
        master_index_address
    );

    current_step = 4;
    invoke_init_callback(
        init_callback,
        InitProgressEvent::Step {
            step: current_step,
            message: "Checking for existing storage...".to_string(),
        },
    )
    .await?;

    let load_result =
        load_master_index_storage_static(&client, &master_index_address, &derived_key).await;

    current_step = 5;
    invoke_init_callback(
        init_callback,
        InitProgressEvent::Step {
            step: current_step,
            message: "Processing storage state...".to_string(),
        },
    )
    .await?;

    let mut existing_loaded = false;

    let mis_mutex = match load_result {
        Ok(mis_arc) => {
            info!("Storage::new: Loaded existing Master Index Storage.");
            existing_loaded = true;

            let mut needs_save = false;
            {
                let mut guard = mis_arc.lock().await;
                if guard.scratchpad_size == 0 {
                    warn!(
                        "Loaded MasterIndexStorage has scratchpad_size 0, setting to default: {}",
                        DEFAULT_SCRATCHPAD_SIZE
                    );
                    guard.scratchpad_size = DEFAULT_SCRATCHPAD_SIZE;
                    needs_save = true;
                }
            }

            if needs_save {
                try_step!(6, "Saving updated storage index", async {
                    storage_save_mis_from_arc_static(
                        &client,
                        &master_index_address,
                        &derived_key,
                        &mis_arc,
                    )
                    .await?;
                    info!("Saved MasterIndexStorage after setting default scratchpad_size.");
                    Ok::<(), Error>(())
                });
            } else {
                current_step = 6;
                invoke_init_callback(
                    init_callback,
                    InitProgressEvent::Step {
                        step: current_step,
                        message: "Storage index up-to-date.".to_string(),
                    },
                )
                .await?;
            }
            mis_arc
        }
        Err(Error::KeyNotFound(_)) => {
            info!(
                "Storage::new: Master Index not found at address {}. Assuming first run...",
                master_index_address
            );
            info!("Using deterministically derived key for first run.");

            let new_mis = MasterIndexStorage::default();
            let mis_arc = Arc::new(Mutex::new(new_mis));

            {
                let mut guard = mis_arc.lock().await;
                if guard.scratchpad_size == 0 {
                    debug!(
                        "Setting default scratchpad size ({}) for new Master Index Storage.",
                        DEFAULT_SCRATCHPAD_SIZE
                    );
                    guard.scratchpad_size = DEFAULT_SCRATCHPAD_SIZE;
                } else {
                    warn!(
                        "New Master Index Storage already had non-zero scratchpad size: {}",
                        guard.scratchpad_size
                    );
                }
            }

            try_step!(6, "Creating initial storage index", async {
                let guard = mis_arc.lock().await;
                let bytes = serde_cbor::to_vec(&*guard)
                    .map_err(|e| Error::SerializationError(e.to_string()))?;

                let content_type = ContentType::MasterIndex as u64;
                let payment_option = PaymentOption::from(&wallet);

                let _created_address = create_scratchpad_static(
                    &client,
                    &derived_key,
                    &bytes,
                    content_type,
                    payment_option,
                )
                .await?;
                Ok::<(), Error>(())
            });

            info!(
                "Storage::new: Successfully created Master Index Storage at {}",
                master_index_address
            );

            mis_arc
        }
        Err(e) => {
            error!("Failed to load Master Index Storage: {}", e);

            invoke_init_callback(
                init_callback,
                InitProgressEvent::Failed {
                    error_msg: e.to_string(),
                },
            )
            .await
            .ok();
            return Err(e);
        }
    };

    let final_event = if existing_loaded {
        InitProgressEvent::ExistingLoaded {
            message: "Existing storage loaded successfully.".to_string(),
        }
    } else {
        InitProgressEvent::Complete {
            message: "Initialization complete.".to_string(),
        }
    };
    invoke_init_callback(init_callback, final_event).await?;

    let storage = Storage {
        wallet,
        client,
        master_index_address,
        master_index_key: derived_key,
    };

    info!("Storage initialization successful.");
    Ok((storage, None, mis_mutex))
}
