use super::network::{
    create_scratchpad_static, load_master_index_storage_static, storage_save_mis_from_arc_static,
};
use super::{ContentType, Storage};
use crate::cache::{read_local_index, write_local_index};
use crate::error::Error;
use crate::events::{invoke_init_callback, InitCallback, InitProgressEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::mutant::NetworkChoice;
use crate::mutant::DEFAULT_SCRATCHPAD_SIZE;
use autonomi::client::payment::PaymentOption;
use autonomi::{Client, ScratchpadAddress, SecretKey, Wallet};
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
    network_choice: NetworkChoice,
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
        match network_choice {
            NetworkChoice::Devnet => {
                info!("Initializing LOCAL network client (Devnet).");
                Client::init_local()
                    .await
                    .map_err(|e| Error::NetworkConnectionFailed(e.to_string()))
            }
            NetworkChoice::Mainnet => {
                info!("Initializing MAIN network client (Mainnet).");
                Client::init()
                    .await
                    .map_err(|e| Error::NetworkConnectionFailed(e.to_string()))
            }
        }
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

        // Create an autonomi::SecretKey directly from the hash bytes array.
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

    // Derive address from the public key corresponding to the derived autonomi::SecretKey
    let derived_public_key = derived_key.public_key();
    let master_index_address = ScratchpadAddress::new(derived_public_key);

    info!(
        "Storage::new: Determined Master Index Address from DERIVED key: {}",
        master_index_address
    );

    // ---> START Cache Check Logic <---
    current_step = 4;
    invoke_init_callback(
        init_callback,
        InitProgressEvent::Step {
            step: current_step,
            message: "Checking local index cache...".to_string(),
        },
    )
    .await?;

    let mut mis_mutex: Option<Arc<Mutex<MasterIndexStorage>>> = None;
    let mut loaded_from_cache = false;

    match read_local_index().await {
        Ok(Some(cached_index)) => {
            info!("Successfully loaded index from local cache.");
            mis_mutex = Some(Arc::new(Mutex::new(cached_index)));
            loaded_from_cache = true;
        }
        Ok(None) => {
            info!("Local cache not found. Will attempt to load from remote.");
        }
        Err(e) => {
            warn!(
                "Failed to read local cache: {}. Will attempt to load from remote.",
                e
            );
        }
    }
    // ---> END Cache Check Logic <---

    // Only attempt remote load if cache was not loaded
    if !loaded_from_cache {
        current_step = 5;
        invoke_init_callback(
            init_callback,
            InitProgressEvent::Step {
                step: current_step,
                message: "Checking for existing remote storage...".to_string(),
            },
        )
        .await?;

        let load_result =
            load_master_index_storage_static(&client, &master_index_address, &derived_key).await;

        current_step = 6;
        invoke_init_callback(
            init_callback,
            InitProgressEvent::Step {
                step: current_step,
                message: "Processing remote storage state...".to_string(),
            },
        )
        .await?;

        mis_mutex = Some(match load_result {
            Ok(mis_arc) => {
                info!("Storage::new: Loaded existing Master Index Storage from remote.");

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
                    try_step!(7, "Saving updated storage index (remote & cache)", async {
                        storage_save_mis_from_arc_static(
                            &client,
                            &master_index_address,
                            &derived_key,
                            &mis_arc,
                        )
                        .await?;
                        info!("Saved MasterIndexStorage remotely after setting default scratchpad_size.");
                        if let Err(e) = write_local_index(&*mis_arc.lock().await).await {
                            warn!("Failed to write updated index to local cache after remote load: {}", e);
                        }
                        Ok::<(), Error>(())
                    });
                } else {
                    if let Err(e) = write_local_index(&*mis_arc.lock().await).await {
                        warn!(
                            "Failed to write index to local cache after remote load: {}",
                            e
                        );
                    }
                    current_step = 7;
                    invoke_init_callback(
                        init_callback,
                        InitProgressEvent::Step {
                            step: current_step,
                            message: "Remote storage index up-to-date.".to_string(),
                        },
                    )
                    .await?;
                }
                mis_arc
            }
            Err(Error::KeyNotFound(_)) => {
                info!(
                    "Storage::new: Remote Master Index not found at address {}. Assuming first run...",
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

                try_step!(
                    7,
                    "Creating initial storage index (remote & cache)",
                    async {
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
                        let index_to_cache = guard.clone();
                        drop(guard);
                        if let Err(e) = write_local_index(&index_to_cache).await {
                            warn!("Failed to write initial index to local cache: {}", e);
                        }
                        Ok::<(), Error>(())
                    }
                );

                info!(
                    "Storage::new: Successfully created Master Index Storage remotely at {}",
                    master_index_address
                );

                mis_arc
            }
            Err(e) => {
                error!("Failed to load Master Index Storage from remote: {}", e);
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
        });
    }

    // Ensure mis_mutex was set either from cache or remote load
    let final_mis_mutex = mis_mutex.ok_or_else(|| {
        error!("Internal error: MasterIndexStorage mutex was not initialized.");
        Error::InternalError("MasterIndexStorage mutex not initialized".to_string())
    })?;

    // Handle the scratchpad_size == 0 case for cache-loaded index
    if loaded_from_cache {
        let mut needs_save = false;
        {
            let mut guard = final_mis_mutex.lock().await;
            if guard.scratchpad_size == 0 {
                warn!(
                    "Loaded MasterIndexStorage from cache has scratchpad_size 0, setting to default: {}",
                    DEFAULT_SCRATCHPAD_SIZE
                );
                guard.scratchpad_size = DEFAULT_SCRATCHPAD_SIZE;
                needs_save = true;
            }
        }
        if needs_save {
            info!("Saving updated index to cache after fixing scratchpad_size.");
            if let Err(e) = write_local_index(&*final_mis_mutex.lock().await).await {
                warn!(
                    "Failed to write updated index to local cache after fixing size: {}",
                    e
                );
            }
        }
    }

    // Adjust total steps reported if cache was loaded
    let reported_total_steps = if loaded_from_cache { 4 } else { total_steps };

    let final_event = if loaded_from_cache {
        InitProgressEvent::ExistingLoaded {
            message: "Existing storage index loaded from local cache.".to_string(),
        }
    } else {
        InitProgressEvent::Complete {
            message: "Initialization complete (loaded from remote).".to_string(),
        }
    };
    invoke_init_callback(
        init_callback,
        InitProgressEvent::Step {
            step: reported_total_steps,
            message: "Finalizing initialization...".to_string(),
        },
    )
    .await?;
    invoke_init_callback(init_callback, final_event).await?;

    let storage = Storage {
        wallet,
        client,
        master_index_address,
        master_index_key: derived_key,
    };

    info!(
        "Storage initialization successful (loaded_from_cache: {}).",
        loaded_from_cache
    );
    Ok((storage, None, final_mis_mutex))
}
