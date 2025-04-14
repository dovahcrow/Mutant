use super::ContentType;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::utils::retry::retry_operation;
use autonomi::{
    client::payment::PaymentOption, Bytes, Client, ScratchpadAddress, SecretKey, Wallet,
};
use log::{debug, error, info, warn};
use serde_cbor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task;

// const VERIFICATION_RETRY_DELAY: Duration = Duration::from_secs(10); // Removed
const VERIFICATION_RETRY_LIMIT: u32 = 360; // 360 retries (was 1 hour timeout with delay)

use crate::mutant::DEFAULT_SCRATCHPAD_SIZE;

// --- Internal Helper ---

/// Fetches and deserializes the MasterIndexStorage, returning KeyNotFound if empty or not found.
async fn _fetch_and_deserialize_mis_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<MasterIndexStorage, Error> {
    match fetch_scratchpad_internal_static(client, address, key).await {
        Ok(bytes) => {
            if bytes.is_empty() {
                warn!("Master Index scratchpad {} exists but is empty.", address);
                // Treat empty as not found, specifically MasterIndexNotFound
                Err(Error::MasterIndexNotFound)
            } else {
                match serde_cbor::from_slice::<MasterIndexStorage>(&bytes) {
                    Ok(mis) => {
                        debug!(
                            "Successfully deserialized MasterIndexStorage from {}.",
                            address
                        );
                        Ok(mis)
                    }
                    Err(e) => {
                        error!(
                            "Failed to deserialize MasterIndexStorage from {}: {}",
                            address, e
                        );
                        Err(Error::Cbor(e)) // Return specific Cbor error
                    }
                }
            }
        }
        Err(Error::KeyNotFound(_)) => {
            debug!("Master Index scratchpad not found at {}.", address);
            // Specific error for not found
            Err(Error::MasterIndexNotFound)
        }
        Err(Error::AutonomiClient(se)) => {
            // Only map RecordNotFound to MasterIndexNotFound, propagate others
            if se.to_string().contains("RecordNotFound") {
                debug!(
                    "Master Index scratchpad RecordNotFound at {} (treating as MasterIndexNotFound).",
                    address
                );
                // Specific error for not found
                Err(Error::MasterIndexNotFound)
            } else {
                error!("Autonomi Error fetching Master Index {}: {}", address, se);
                Err(Error::AutonomiClient(se))
            }
        }
        Err(e) => {
            // Propagate other errors (JoinError, etc.)
            error!("Unexpected error during MIS fetch {}: {}", address, e);
            Err(e)
        }
    }
}

// --- Public Crate Functions ---

pub(crate) async fn load_master_index_storage_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<Arc<Mutex<MasterIndexStorage>>, Error> {
    debug!(
        "load_master_index_storage_static: Attempting to load/create MIS from address {}...",
        address
    );
    match _fetch_and_deserialize_mis_static(client, address, key).await {
        Ok(mis) => {
            // Successfully fetched and deserialized
            Ok(Arc::new(Mutex::new(mis)))
        }
        Err(Error::MasterIndexNotFound) => {
            // Specifically handle MasterIndexNotFound - DO NOT create default here anymore.
            // The caller (MutAnt::init) will now handle this case and potentially prompt the user.
            debug!("load_master_index_storage_static: Master index not found at {}. Returning specific error.", address);
            Err(Error::MasterIndexNotFound)
        }
        Err(Error::Cbor(e)) => {
            // Invalid format, still create default locally but log error
            error!("load_master_index_storage_static: Found index at {} but failed to deserialize: {}. Creating default.", address, e);
            let default_mis = MasterIndexStorage {
                scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                ..Default::default()
            };
            // Do NOT attempt to save the default back in this specific error case.
            Ok(Arc::new(Mutex::new(default_mis)))
        }
        Err(e) => {
            // Propagate other errors (AutonomiClient other than RecordNotFound, JoinError, etc.)
            error!(
                "load_master_index_storage_static: Unexpected error fetching/deserializing MIS {}: {}",
                address, e
            );
            Err(e)
        }
    }
}

pub(crate) async fn storage_save_mis_from_arc_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
    mis_arc: &Arc<Mutex<MasterIndexStorage>>,
) -> Result<(), Error> {
    debug!("Attempting to save Master Index Storage to {}...", address);
    let mis_guard = mis_arc.lock().await;
    let state = &*mis_guard;
    let bytes = serde_cbor::to_vec(state).map_err(|e| Error::SerializationError(e.to_string()))?;
    debug!(
        "storage_save_mis_from_arc_static: Serialized MIS ({} bytes) for {}. Preparing to write.",
        bytes.len(),
        address
    );

    // Call the wrapper function that handles non-progress updates
    let update_result =
        update_scratchpad_internal_static(client, key, &bytes, ContentType::MasterIndex as u64)
            .await;

    match &update_result {
        Ok(_) => debug!(
            "storage_save_mis_from_arc_static: update_scratchpad_internal_static succeeded for {}.",
            address
        ),
        Err(e) => error!(
            "storage_save_mis_from_arc_static: update_scratchpad_internal_static failed for {}: {}",
            address, e
        ),
    }
    update_result?; // Propagate error if it failed

    debug!("Successfully saved MasterIndexStorage to {}.", address);
    Ok(())
}

/// Creates the Master Index Storage scratchpad for the first time.
/// Serializes the provided MIS and uses `create_scratchpad_static`.
pub(crate) async fn storage_create_mis_from_arc_static(
    client: &Client,
    wallet: &Wallet,
    address: &ScratchpadAddress,
    key: &SecretKey,
    mis_arc: &Arc<Mutex<MasterIndexStorage>>,
) -> Result<(), Error> {
    debug!(
        "Attempting to create initial Master Index Storage at {}...",
        address
    );
    let mis_guard = mis_arc.lock().await;
    let state = &*mis_guard;
    let bytes = serde_cbor::to_vec(state).map_err(|e| Error::SerializationError(e.to_string()))?;
    drop(mis_guard); // Release lock before network call

    debug!(
        "storage_create_mis_from_arc_static: Serialized MIS ({} bytes) for {}. Preparing to create.",
        bytes.len(),
        address
    );

    // Use create_scratchpad_static instead of update
    let payment_option = PaymentOption::from(wallet);
    let dummy_bytes_uploaded: Arc<AtomicU64> = Arc::new(AtomicU64::new(0)); // Use AtomicU64
    let data_len = bytes.len() as u64; // total size for MIS
    let create_result = create_scratchpad_static(
        client,
        key,
        &bytes,
        ContentType::MasterIndex as u64,
        payment_option,
        "_internal_update_",         // key_str (internal context)
        0,                           // pad_index (internal context)
        &Arc::new(Mutex::new(None)), // dummy_callback
        &Arc::new(Mutex::new(0)),    // dummy_pads_committed
        1,                           // total_pads_expected (assume 1 for MIS)
        &dummy_bytes_uploaded,       // Pass Arc<AtomicU64>
        data_len,                    // Pass total size
        &Arc::new(Mutex::new(0)),    // Pass dummy create counter
    )
    .await;

    match &create_result {
        Ok(created_addr) => {
            // Verify the address matches the expected one derived from the key
            if created_addr != address {
                error!(
                    "storage_create_mis_from_arc_static: Mismatch! Expected address {} but creation returned {}.",
                    address, created_addr
                );
                Err(Error::InternalError(format!(
                    "Created MIS address {} does not match expected address {}",
                    created_addr, address
                )))
            } else {
                debug!(
                    "storage_create_mis_from_arc_static: create_scratchpad_static succeeded for {}.",
                    address
                );
                Ok(()) // Return Ok(()) on success
            }
        }
        Err(e) => {
            error!(
                "storage_create_mis_from_arc_static: create_scratchpad_static failed for {}: {}",
                address, e
            );
            // Map the error instead of cloning
            Err(Error::InternalError(format!(
                "Failed to create scratchpad {}: {}",
                address,
                e.to_string()
            )))
        }
    }
}

pub(crate) async fn fetch_scratchpad_internal_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<Vec<u8>, Error> {
    let start_time = std::time::Instant::now();
    debug!("FetchStatic[{}]: Starting fetch operation...", address);

    let result = retry_operation(
        &format!("Fetch scratchpad {}", address),
        || async {
            let fetch_start_time = std::time::Instant::now();
            let scratchpad_result = client.scratchpad_get(address).await;
            let fetch_duration = fetch_start_time.elapsed();

            let scratchpad = match scratchpad_result {
                Ok(s) => s,
                Err(e) => {
                    debug!(
                        "FetchStatic[{}]: client.scratchpad_get failed after {:?}: {}",
                        address, fetch_duration, e
                    );

                    if e.to_string().contains("RecordNotFound") {
                        return Err(Error::KeyNotFound(address.to_string()));
                    } else {
                        return Err(Error::AutonomiClient(e));
                    }
                }
            };

            let raw_data_len = scratchpad.payload_size();
            debug!(
                "FetchStatic[{}]: client.scratchpad_get SUCCEEDED after {:?}. Raw data size: {}",
                address, fetch_duration, raw_data_len
            );

            let decrypt_start_time = std::time::Instant::now();

            let scratchpad_clone = scratchpad.clone();
            let key_clone = key.clone();
            let address_clone = address.clone();

            let decrypted_bytes_result = task::spawn_blocking(move || {
                debug!(
                    "FetchStatic[{}]: Entering spawn_blocking for decryption...",
                    address_clone
                );
                let res = scratchpad_clone.decrypt_data(&key_clone);
                debug!(
                    "FetchStatic[{}]: Exiting spawn_blocking for decryption. Success: {}",
                    address_clone,
                    res.is_ok()
                );
                res
            })
            .await;

            let decrypt_duration = decrypt_start_time.elapsed();

            let decrypted_bytes: Bytes = match decrypted_bytes_result {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => {
                    error!(
                        "FetchStatic[{}]: Decryption failed inside spawn_blocking after {:?}: {}",
                        address, decrypt_duration, e
                    );
                    return Err(Error::AutonomiLibError(format!(
                        "Decryption failed for {}: {}",
                        address, e
                    )));
                }
                Err(join_error) => {
                    error!(
                        "FetchStatic[{}]: spawn_blocking for decryption failed after {:?}: {}",
                        address, decrypt_duration, join_error
                    );
                    return Err(Error::from_join_error_msg(
                        &join_error,
                        format!("Decryption task failed for {}", address),
                    ));
                }
            };

            let decrypted_len = decrypted_bytes.len();
            debug!(
                "FetchStatic[{}]: Decryption SUCCEEDED after {:?}. Decrypted data size: {}",
                address, decrypt_duration, decrypted_len
            );

            Ok(decrypted_bytes.to_vec())
        },
        |e: &Error| {
            matches!(e, Error::AutonomiClient(s) if !s.to_string().contains("RecordNotFound"))
                || matches!(e, Error::TaskJoinError(_))
        },
    )
    .await;

    let total_duration = start_time.elapsed();
    match &result {
        Ok(data) => {
            debug!(
                "FetchStatic[{}]: Total fetch_scratchpad_internal_static duration: {:?}, returning {} bytes.",
                address,
                total_duration,
                data.len()
            );
        }
        Err(e) => {
            debug!(
                "FetchStatic[{}]: Total fetch_scratchpad_internal_static duration: {:?}, returning error: {}",
                address, total_duration, e
            );
        }
    }

    result
}

/// Internal helper with progress reporting arguments
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_scratchpad_internal_static_with_progress(
    client: &Client,
    owner_key: &SecretKey,
    data: &[u8],
    content_type: u64,
    key_str: &str,
    pad_index: usize,
    callback_arc: &Arc<tokio::sync::Mutex<Option<PutCallback>>>,
    total_bytes_uploaded_arc: &Arc<AtomicU64>,
    bytes_in_chunk: u64,
    total_size_overall: u64,
    is_newly_reserved: bool,
    total_pads_committed_arc: &Arc<tokio::sync::Mutex<u64>>,
    total_pads_expected: usize,
) -> Result<(), Error> {
    let data_bytes = Bytes::from(data.to_vec());
    let owner_pubkey = owner_key.public_key();
    let address = ScratchpadAddress::new(owner_pubkey.into());
    let data_len = data.len(); // For logging

    // Get initial state for verification later
    let initial_counter = match client.scratchpad_get(&address).await {
        Ok(s) => Some(s.counter()),
        Err(e) => {
            warn!(
                "UpdateStaticVerify[{}][{}][Pad {}]: Failed to get initial counter before update: {}. Proceeding with update.",
                key_str, address, pad_index, e
            );
            None // May not exist yet, or network error
        }
    };

    debug!(
        "UpdateStatic[{}][{}][Pad {}]: Starting update operation. Data size: {}, Content type: {}",
        key_str, address, pad_index, data_len, content_type
    );

    retry_operation(
        &format!("Update scratchpad {} ({})", key_str, address),
        || async {
            client
                .scratchpad_update(owner_key, content_type, &data_bytes)
                .await
                .map_err(Error::AutonomiClient)
        },
        |_e: &Error| true,
    )
    .await?;

    info!(
        "UpdateStatic[{}][{}][Pad {}]: Network update call successful. Proceeding to verification prep.",
        key_str, address, pad_index
    );

    // --- Emit UploadProgress ---
    let current_total_bytes =
        total_bytes_uploaded_arc.fetch_add(bytes_in_chunk, Ordering::Relaxed) + bytes_in_chunk;

    debug!(
        "UpdateStatic[{}][{}][Pad {}]: Upload progress updated: {} / {}",
        key_str, address, pad_index, current_total_bytes, total_size_overall
    );
    {
        // Scope for callback lock
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(
            &mut *cb_guard,
            PutEvent::UploadProgress {
                bytes_written: current_total_bytes,
                total_bytes: total_size_overall,
            },
        )
        .await?;
    } // Callback lock released
      // --- End Emit UploadProgress ---

    // --- Add delay before starting verification ---
    debug!(
        "UpdateStatic[{}][{}][Pad {}]: Waiting 5 seconds before starting verification loop...",
        key_str, address, pad_index
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    // ------------------------------------------

    debug!(
        "UpdateStatic[{}][{}][Pad {}]: Starting verification loop...",
        key_str, address, pad_index
    );

    for attempt in 0..VERIFICATION_RETRY_LIMIT {
        // Introduce delay *before* the attempt (except the first one)
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        debug!(
            "UpdateStaticVerify[{}][{}][Pad {}]: Verification attempt {}/{}...",
            key_str,
            address,
            pad_index,
            attempt + 1,
            VERIFICATION_RETRY_LIMIT
        );

        match client.scratchpad_get(&address).await {
            Ok(scratchpad) => {
                debug!(
                    "UpdateStaticVerify[{}][{}][Pad {}]: Fetched scratchpad. Counter: {}, PayloadSize: {}",
                    key_str,
                    address,
                    pad_index,
                    scratchpad.counter(),
                    scratchpad.payload_size()
                );

                // 1. Check counter increment (if initial counter was available)
                let counter_check_passed = match initial_counter {
                    Some(initial) => scratchpad.counter() > initial,
                    None => true, // Cannot verify counter if initial fetch failed
                };

                if !counter_check_passed {
                    debug!(
                        "UpdateStaticVerify[{}][{}][Pad {}]: Counter check failed (Initial: {:?}, Current: {}). Retrying...",
                        key_str, address, pad_index, initial_counter, scratchpad.counter()
                    );
                    continue; // Counter hasn't incremented yet
                }

                // 3. Decrypt and compare data (Now step 2)
                debug!(
                    "UpdateStaticVerify[{}][{}][Pad {}]: Counter check passed. Proceeding to decryption check.",
                    key_str, address, pad_index
                );
                let key_clone = owner_key.clone();
                // let address_clone = address.clone(); // Unused, decryption errors use `address`
                match task::spawn_blocking(move || scratchpad.decrypt_data(&key_clone)).await {
                    Ok(Ok(decrypted_bytes)) => {
                        if decrypted_bytes.as_ref() == data {
                            debug!(
                                "UpdateStaticVerify[{}][{}][Pad {}]: Verification successful on attempt {}.",
                                key_str, address, pad_index, attempt + 1
                            );

                            // --- Emit PadConfirmed (ALWAYS emit on successful verification) ---
                            let current_confirmed = {
                                let mut counter = total_pads_committed_arc.lock().await;
                                *counter += 1;
                                debug!(
                                    "UpdateStaticVerify[{}][{}][Pad {}]: Incrementing commit counter to {}. Emitting PadConfirmed.",
                                    key_str, address, pad_index, *counter
                                );
                                *counter // Return the incremented value
                            };

                            invoke_callback(
                                &mut *callback_arc.lock().await,
                                PutEvent::PadConfirmed {
                                    current: current_confirmed,
                                    total: total_pads_expected as u64,
                                },
                            )
                            .await?;

                            return Ok(()); // Verification successful
                        } else {
                            debug!(
                                "UpdateStaticVerify[{}][{}][Pad {}]: Data mismatch during verification attempt {} (Expected len: {}, Got len: {}). Retrying...",
                                key_str,
                                address,
                                pad_index,
                                attempt + 1,
                                data.len(),
                                decrypted_bytes.len()
                            );
                            continue; // Data doesn't match yet
                        }
                    }
                    Ok(Err(e)) => {
                        // Decryption error might be transient right after update
                        debug!(
                            "UpdateStaticVerify[{}][{}][Pad {}]: Decryption failed during verification attempt {} (likely transient): {}. Retrying...",
                             key_str, address, pad_index, attempt + 1, e
                        );
                        // return Err(Error::AutonomiLibError(format!("Verification decryption failed: {}", e))); // Don't abort, retry
                        continue;
                    }
                    Err(join_error) => {
                        // Task failure is serious
                        error!(
                            "UpdateStaticVerify[{}][{}][Pad {}]: Spawn_blocking failed during verification: {}. Aborting verification.",
                             key_str, address, pad_index, join_error
                        );
                        return Err(Error::from_join_error_msg(
                            &join_error,
                            format!("Verification task failed for {}[{}]", key_str, address),
                        ));
                    }
                }
            }
            Err(e) => {
                if e.to_string().contains("RecordNotFound") {
                    debug!(
                        "UpdateStaticVerify[{}][{}][Pad {}]: RecordNotFound during verification attempt {}. Retrying...",
                        key_str, address, pad_index, attempt + 1
                    );
                    // Continue loop, waiting for propagation
                } else {
                    error!(
                        "UpdateStaticVerify[{}][{}][Pad {}]: Unexpected error during verification attempt {}: {}. Aborting verification.",
                        key_str, address, pad_index, attempt + 1, e
                    );
                    return Err(Error::AutonomiClient(e)); // Propagate other client errors
                }
            }
        }
    }

    error!(
        "UpdateStaticVerify[{}][{}][Pad {}]: Verification timed out after {} attempts.",
        key_str, address, pad_index, VERIFICATION_RETRY_LIMIT
    );
    Err(Error::VerificationTimeout(format!(
        "Update verification timed out for {} ({})",
        key_str, address
    )))
}

/// Public wrapper for internal updates that don't need progress reporting (e.g., MIS save).
/// Calls the _with_progress version with dummy/None values.
pub(crate) async fn update_scratchpad_internal_static(
    client: &Client,
    owner_key: &SecretKey,
    data: &[u8],
    content_type: u64,
) -> Result<(), Error> {
    // Create dummy values for progress reporting context
    let dummy_callback: Arc<tokio::sync::Mutex<Option<PutCallback>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let dummy_bytes_uploaded: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let dummy_pads_committed: Arc<tokio::sync::Mutex<u64>> = Arc::new(tokio::sync::Mutex::new(0));
    let data_len = data.len() as u64;

    // Call the detailed function
    update_scratchpad_internal_static_with_progress(
        client,
        owner_key,
        data,
        content_type,
        "_internal_update_", // key_str (internal context)
        0,                   // pad_index (internal context)
        &dummy_callback,     // Use ref to Arc
        &dummy_bytes_uploaded,
        data_len, // bytes_in_chunk (assume single chunk for MIS)
        data_len, // total_size_overall
        false,    // is_newly_reserved (MIS update never counts as new user pad)
        &dummy_pads_committed,
        1, // total_pads_expected (assume 1 for MIS)
    )
    .await
}

// Create scratchpad and perform verification
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_scratchpad_static(
    client: &Client,
    owner_key: &SecretKey,
    initial_data: &[u8],
    content_type: u64,
    payment_option: PaymentOption,
    key_str: &str,
    pad_index: usize,
    callback_arc: &Arc<tokio::sync::Mutex<Option<PutCallback>>>,
    total_pads_committed_arc: &Arc<tokio::sync::Mutex<u64>>,
    total_pads_expected: usize,
    total_bytes_uploaded_arc: &Arc<AtomicU64>,
    total_size_overall: u64,
    create_counter_arc: &Arc<tokio::sync::Mutex<u64>>,
) -> Result<ScratchpadAddress, Error> {
    let data_bytes = Bytes::from(initial_data.to_vec());
    let expected_address = ScratchpadAddress::new(owner_key.public_key().into());
    let initial_data_len = initial_data.len();
    let bytes_in_chunk = initial_data_len as u64;

    debug!(
        "CreateStatic[{}][{}][Pad {}]: Attempting creation. Data size: {}, ContentType: {}",
        key_str, expected_address, pad_index, initial_data_len, content_type,
    );

    let creation_result = retry_operation(
        &format!("Static Create Scratchpad {} Pad {}", key_str, pad_index),
        || {
            let client_clone = client.clone();
            let owner_key_clone = owner_key.clone();
            let data_bytes_clone = data_bytes.clone();
            let payment_option_clone = payment_option.clone();
            let expected_address_clone = expected_address.clone(); // Clone for closure
            let key_str_clone = key_str.to_string(); // Clone for closure
            let pad_index_clone = pad_index;         // Copy (usize is Copy)
            async move {
                match client_clone
                    .scratchpad_create(
                        &owner_key_clone,
                        content_type,
                        &data_bytes_clone,
                        payment_option_clone,
                    )
                    .await
                {
                    Ok((_cost, address)) => Ok(address),
                    Err(e) => {
                        // Check if the error indicates the scratchpad already exists
                        let error_string = e.to_string();
                        if error_string.contains("already exists") || error_string.contains("RecordAlreadyExists") { // Adjust based on exact error message
                            warn!(
                                "CreateStatic[{}][{}][Pad {}]: Attempted to create scratchpad, but it already exists. Treating as success.",
                                key_str_clone, expected_address_clone, pad_index_clone
                            );
                            Ok(expected_address_clone) // Return expected address if it already exists
                        } else {
                            Err(Error::AutonomiClient(e)) // Propagate other client errors
                        }
                    }
                }
            }
        },
        |e: &Error| {
            // Retry on most AutonomiClient errors, *except* the "already exists" case which we now handle.
            // Also retry on JoinErrors etc.
             match e {
                Error::AutonomiClient(se) => {
                    let error_string = se.to_string();
                     !(error_string.contains("already exists") || error_string.contains("RecordAlreadyExists"))
                }
                _ => true, // Retry on other error types like JoinError
            }
        },
    )
    .await;

    let created_address = match creation_result {
        Ok(addr) => addr,
        Err(e) => {
            error!(
                "CreateStatic[{}][{}][Pad {}]: Failed to create scratchpad after retries: {}",
                key_str, expected_address, pad_index, e
            );
            return Err(e); // Return the final error after retries
        }
    };

    info!(
        "CreateStatic[{}][{}][Pad {}]: Network create call successful (or pad already existed). Address: {}",
        key_str, expected_address, pad_index, created_address
    );

    if created_address != expected_address {
        error!(
            "FATAL: CreateStatic[{}][{}][Pad {}]: Created address {} mismatch derived {}. Aborting.",
            key_str, expected_address, pad_index, created_address, expected_address
        );
        return Err(Error::InternalError(
            "Mismatch between derived and created scratchpad address".to_string(),
        ));
    }

    // --- Emit PadCreateSuccess ---
    let current_created = {
        let mut counter = create_counter_arc.lock().await;
        *counter += 1;
        *counter // Return incremented value
    };
    {
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(
            &mut *cb_guard,
            PutEvent::PadCreateSuccess {
                current: current_created,
                total: total_pads_expected as u64,
            },
        )
        .await?;
    }
    debug!(
        "CreateStatic[{}][{}][Pad {}]: Emitted PadCreateSuccess (Current: {}, Total: {}).",
        key_str, expected_address, pad_index, current_created, total_pads_expected
    );

    // --- Emit UploadProgress ---
    let current_total_bytes =
        total_bytes_uploaded_arc.fetch_add(bytes_in_chunk, Ordering::Relaxed) + bytes_in_chunk;

    {
        let mut cb_guard = callback_arc.lock().await;
        invoke_callback(
            &mut *cb_guard,
            PutEvent::UploadProgress {
                bytes_written: current_total_bytes,
                total_bytes: total_size_overall,
            },
        )
        .await?;
    }
    debug!(
        "CreateStatic[{}][{}][Pad {}]: Emitted UploadProgress ({} / {}).",
        key_str, expected_address, pad_index, current_total_bytes, total_size_overall
    );

    // --- Start Verification Loop (Adapted from update_...) ---
    debug!(
        "CreateStatic[{}][{}][Pad {}]: Waiting 5 seconds before starting verification loop...",
        key_str, expected_address, pad_index
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    debug!(
        "CreateStatic[{}][{}][Pad {}]: Starting verification loop...",
        key_str, expected_address, pad_index
    );

    for attempt in 0..VERIFICATION_RETRY_LIMIT {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        debug!(
            "CreateStaticVerify[{}][{}][Pad {}]: Verification attempt {}/{}...",
            key_str,
            expected_address,
            pad_index,
            attempt + 1,
            VERIFICATION_RETRY_LIMIT
        );

        match client.scratchpad_get(&expected_address).await {
            Ok(scratchpad) => {
                debug!(
                    "CreateStaticVerify[{}][{}][Pad {}]: Fetched scratchpad. Counter: {}, PayloadSize: {}",
                    key_str,
                    expected_address,
                    pad_index,
                    scratchpad.counter(),
                    scratchpad.payload_size()
                );

                // 2. Decrypt and compare data
                debug!(
                    "CreateStaticVerify[{}][{}][Pad {}]: Proceeding to decryption check (Counter check removed).",
                    key_str, expected_address, pad_index
                );
                let key_clone = owner_key.clone();
                match task::spawn_blocking(move || scratchpad.decrypt_data(&key_clone)).await {
                    Ok(Ok(decrypted_bytes)) => {
                        if decrypted_bytes.as_ref() == initial_data {
                            debug!(
                                "CreateStaticVerify[{}][{}][Pad {}]: Verification successful on attempt {}.",
                                key_str, expected_address, pad_index, attempt + 1
                            );

                            // --- Emit PadConfirmed --- //
                            let current_confirmed = {
                                let mut counter = total_pads_committed_arc.lock().await;
                                *counter += 1;
                                debug!(
                                    "CreateStaticVerify[{}][{}][Pad {}]: Incrementing commit counter to {}. Emitting PadConfirmed.",
                                    key_str, expected_address, pad_index, *counter
                                );
                                *counter
                            };
                            invoke_callback(
                                &mut *callback_arc.lock().await,
                                PutEvent::PadConfirmed {
                                    current: current_confirmed,
                                    total: total_pads_expected as u64,
                                },
                            )
                            .await?;
                            return Ok(expected_address); // Verification successful
                        } else {
                            debug!(
                                "CreateStaticVerify[{}][{}][Pad {}]: Data mismatch during verification attempt {} (Expected len: {}, Got len: {}). Retrying...",
                                key_str,
                                expected_address,
                                pad_index,
                                attempt + 1,
                                initial_data.len(),
                                decrypted_bytes.len()
                            );
                            continue;
                        }
                    }
                    Ok(Err(e)) => {
                        debug!(
                            "CreateStaticVerify[{}][{}][Pad {}]: Decryption failed during verification attempt {} (likely transient): {}. Retrying...",
                             key_str, expected_address, pad_index, attempt + 1, e
                        );
                        continue;
                    }
                    Err(join_error) => {
                        error!(
                            "CreateStaticVerify[{}][{}][Pad {}]: Spawn_blocking failed during verification: {}. Aborting verification.",
                             key_str, expected_address, pad_index, join_error
                        );
                        return Err(Error::from_join_error_msg(
                            &join_error,
                            format!(
                                "Verification task failed for {}[{}]",
                                key_str, expected_address
                            ),
                        ));
                    }
                }
            }
            Err(e) => {
                if e.to_string().contains("RecordNotFound") {
                    debug!(
                        "CreateStaticVerify[{}][{}][Pad {}]: RecordNotFound during verification attempt {}. Retrying...",
                        key_str, expected_address, pad_index, attempt + 1
                    );
                    // Continue loop, waiting for propagation
                } else {
                    error!(
                        "CreateStaticVerify[{}][{}][Pad {}]: Unexpected error during verification attempt {}: {}. Aborting verification.",
                        key_str, expected_address, pad_index, attempt + 1, e
                    );
                    return Err(Error::AutonomiClient(e));
                }
            }
        }
    }

    error!(
        "CreateStaticVerify[{}][{}][Pad {}]: Verification timed out after {} attempts.",
        key_str, expected_address, pad_index, VERIFICATION_RETRY_LIMIT
    );
    Err(Error::VerificationTimeout(format!(
        "Create verification timed out for {} ({})",
        key_str, expected_address
    )))
}

/// Fetches and deserializes the MasterIndexStorage from the network without cache interaction.
pub(crate) async fn fetch_remote_master_index_storage_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<MasterIndexStorage, Error> {
    debug!(
        "fetch_remote_master_index_storage_static: Attempting fetch from {}...",
        address
    );
    // Just call the shared helper and return its result directly
    _fetch_and_deserialize_mis_static(client, address, key).await
}
