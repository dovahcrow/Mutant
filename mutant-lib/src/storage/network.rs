use super::ContentType;
use crate::error::Error;
use crate::mutant::data_structures::MasterIndexStorage;
use crate::mutant::MASTER_INDEX_KEY;
use crate::utils::retry::retry_operation;
use autonomi::{client::payment::PaymentOption, Bytes, Client, ScratchpadAddress, SecretKey};
use log::{debug, error, warn};
use serde_cbor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task;

// const VERIFICATION_RETRY_DELAY: Duration = Duration::from_secs(10); // Removed
const VERIFICATION_RETRY_LIMIT: u32 = 360; // 360 retries (was 1 hour timeout with delay)

pub(crate) async fn load_master_index_storage_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<Arc<Mutex<MasterIndexStorage>>, Error> {
    debug!(
        "Attempting to load Master Index Storage from address {} using provided key...",
        address
    );
    match fetch_scratchpad_internal_static(client, address, key).await {
        Ok(bytes) => {
            if bytes.is_empty() {
                warn!("Master Index scratchpad {} exists but is empty.", address);
                return Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()));
            }
            match serde_cbor::from_slice::<MasterIndexStorage>(&bytes) {
                Ok(mis) => {
                    debug!(
                        "Successfully deserialized MasterIndexStorage from {}.",
                        address
                    );
                    Ok(Arc::new(Mutex::new(mis)))
                }
                Err(e) => {
                    error!(
                        "Failed to deserialize MasterIndexStorage from {}: {}",
                        address, e
                    );

                    Err(Error::KeyNotFound(format!(
                        "Deserialization failed for {}: {}",
                        address, e
                    )))
                }
            }
        }
        Err(Error::KeyNotFound(_)) => {
            debug!("Master Index scratchpad not found at {}.", address);
            Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()))
        }
        Err(Error::AutonomiClient(se)) => {
            error!("Autonomi Error fetching Master Index {}: {}", address, se);
            if se.to_string().contains("RecordNotFound") {
                Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()))
            } else {
                Err(Error::AutonomiClient(se))
            }
        }
        Err(e) => {
            error!("Error fetching Master Index {}: {}", address, e);
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

pub(crate) async fn update_scratchpad_internal_static(
    client: &Client,
    owner_key: &SecretKey,
    data: &[u8],
    content_type: u64,
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
                "UpdateStaticVerify[{}]: Failed to get initial counter before update: {}. Proceeding with update.",
                address, e
            );
            None // May not exist yet, or network error
        }
    };

    debug!(
        "UpdateStatic[{}]: Starting update operation. Data size: {}, Content type: {}",
        address, data_len, content_type
    );

    retry_operation(
        &format!("Update scratchpad {}", address),
        || async {
            client
                .scratchpad_update(owner_key, content_type, &data_bytes)
                .await
                .map_err(Error::AutonomiClient)
        },
        |_e: &Error| true,
    )
    .await?;

    debug!(
        "UpdateStatic[{}]: Update operation successful. Starting verification loop ({} attempts, no delay).",
        address, VERIFICATION_RETRY_LIMIT
    );

    for attempt in 0..VERIFICATION_RETRY_LIMIT {
        // Introduce delay *before* the attempt (except the first one)
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        debug!(
            "UpdateStaticVerify[{}]: Verification attempt {}/{}...",
            address,
            attempt + 1,
            VERIFICATION_RETRY_LIMIT
        );

        match client.scratchpad_get(&address).await {
            Ok(scratchpad) => {
                debug!(
                    "UpdateStaticVerify[{}]: Fetched scratchpad. Counter: {}, PayloadSize: {}",
                    address,
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
                        "UpdateStaticVerify[{}]: Counter check failed (Initial: {:?}, Current: {}). Retrying...",
                        address, initial_counter, scratchpad.counter()
                    );
                    continue; // Counter hasn't incremented yet
                }

                // 3. Decrypt and compare data (Now step 2)
                debug!(
                    "UpdateStaticVerify[{}]: Counter check passed. Proceeding to decryption check.",
                    address
                );
                let key_clone = owner_key.clone();
                // let address_clone = address.clone(); // Unused, decryption errors use `address`
                match task::spawn_blocking(move || scratchpad.decrypt_data(&key_clone)).await {
                    Ok(Ok(decrypted_bytes)) => {
                        if decrypted_bytes.as_ref() == data {
                            debug!(
                                "UpdateStaticVerify[{}]: Verification successful on attempt {}.",
                                address,
                                attempt + 1
                            );
                            return Ok(()); // Data matches!
                        } else {
                            debug!(
                                "UpdateStaticVerify[{}]: Data mismatch during verification attempt {} (Expected len: {}, Got len: {}). Retrying...",
                                address,
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
                            "UpdateStaticVerify[{}]: Decryption failed during verification attempt {} (likely transient): {}. Retrying...",
                             address, attempt + 1, e
                        );
                        // return Err(Error::AutonomiLibError(format!("Verification decryption failed: {}", e))); // Don't abort, retry
                        continue;
                    }
                    Err(join_error) => {
                        // Task failure is serious
                        error!(
                            "UpdateStaticVerify[{}]: Spawn_blocking failed during verification: {}. Aborting verification.",
                             address, join_error
                        );
                        return Err(Error::from_join_error_msg(
                            &join_error,
                            format!("Verification task failed for {}", address),
                        ));
                    }
                }
            }
            Err(e) => {
                if e.to_string().contains("RecordNotFound") {
                    debug!(
                        "UpdateStaticVerify[{}]: RecordNotFound during verification attempt {}. Retrying...",
                        address, attempt + 1
                    );
                    // Continue loop, waiting for propagation
                } else {
                    error!(
                        "UpdateStaticVerify[{}]: Unexpected error during verification attempt {}: {}. Aborting verification.",
                        address, attempt + 1, e
                    );
                    return Err(Error::AutonomiClient(e)); // Propagate other client errors
                }
            }
        }
    }

    error!(
        "UpdateStaticVerify[{}]: Verification timed out after {} attempts.",
        address, VERIFICATION_RETRY_LIMIT
    );
    Err(Error::VerificationTimeout(format!(
        "Update verification timed out for {}",
        address
    )))
}

pub(crate) async fn create_scratchpad_static(
    client: &Client,
    owner_key: &SecretKey,
    initial_data: &[u8],
    content_type: u64,
    payment_option: PaymentOption,
) -> Result<ScratchpadAddress, Error> {
    let data_bytes = Bytes::from(initial_data.to_vec());
    let expected_address = ScratchpadAddress::new(owner_key.public_key().into());
    let initial_data_len = initial_data.len(); // For logging

    debug!(
        "CreateStatic[{}]: Attempting static scratchpad creation for owner {:?} at expected address {}. Data size: {}, ContentType: {}",
        expected_address,
        owner_key.public_key(),
        expected_address,
        initial_data_len,
        content_type,
    );

    let created_address = retry_operation(
        &format!("Static Create Scratchpad ({})", expected_address),
        || {
            let client_clone = client.clone();
            let owner_key_clone = owner_key.clone();
            let data_bytes_clone = data_bytes.clone();
            let payment_option_clone = payment_option.clone();
            async move {
                let (_cost, address) = client_clone
                    .scratchpad_create(
                        &owner_key_clone,
                        content_type,
                        &data_bytes_clone,
                        payment_option_clone,
                    )
                    .await
                    .map_err(Error::AutonomiClient)?;
                Ok(address)
            }
        },
        |_e: &Error| true, // Retry on all errors for creation
    )
    .await?;

    if created_address != expected_address {
        error!(
            "FATAL: Created scratchpad address {} does not match address derived from owner key {}. This should not happen.",
            created_address, expected_address
        );
        return Err(Error::InternalError(
            "Mismatch between derived and created scratchpad address".to_string(),
        ));
    }
    debug!(
        "CreateStatic[{}]: Static scratchpad creation reported successful. Starting verification loop ({} attempts, no delay).",
        created_address, VERIFICATION_RETRY_LIMIT
    );

    // Verification Loop - Check only for existence, not content (decryption fails on newly created empty pads)
    for attempt in 0..VERIFICATION_RETRY_LIMIT {
        debug!(
            "CreateStaticVerify[{}]: Verification attempt {}/{} (checking existence)...",
            created_address,
            attempt + 1,
            VERIFICATION_RETRY_LIMIT
        );

        match client.scratchpad_get(&created_address).await {
            Ok(_) => {
                debug!(
                    "CreateStaticVerify[{}]: Existence verified on attempt {}.",
                    created_address,
                    attempt + 1
                );
                return Ok(created_address); // Success! Record exists.
            }
            Err(e) => {
                let e_str = e.to_string();
                if e_str.contains("RecordNotFound") {
                    debug!(
                        "CreateStaticVerify[{}]: RecordNotFound during existence check attempt {}. Retrying...",
                        created_address, attempt + 1
                    );
                    // Continue loop, waiting for propagation
                } else {
                    error!(
                        "CreateStaticVerify[{}]: Unexpected error during existence check attempt {}: {}. Aborting verification.",
                        created_address, attempt + 1, e
                    );
                    return Err(Error::AutonomiClient(e)); // Propagate other client errors
                }
            }
        }
    }

    error!(
        "CreateStaticVerify[{}]: Verification timed out after {} attempts.",
        created_address, VERIFICATION_RETRY_LIMIT
    );
    Err(Error::VerificationTimeout(format!(
        "Create verification timed out for {}",
        created_address
    )))
}

/// Fetches and deserializes the MasterIndexStorage from the network without cache interaction.
pub(crate) async fn fetch_remote_master_index_storage_static(
    client: &Client,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<MasterIndexStorage, Error> {
    debug!(
        "Attempting to fetch REMOTE Master Index Storage from address {} using provided key...",
        address
    );
    match fetch_scratchpad_internal_static(client, address, key).await {
        Ok(bytes) => {
            if bytes.is_empty() {
                warn!(
                    "REMOTE Master Index scratchpad {} exists but is empty.",
                    address
                );
                // Return a default/empty index in this case, as the scratchpad exists but is unusable.
                // Or should this be an error? Let's treat it as effectively not found for now.
                return Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()));
            }
            match serde_cbor::from_slice::<MasterIndexStorage>(&bytes) {
                Ok(mis) => {
                    debug!(
                        "Successfully deserialized REMOTE MasterIndexStorage from {}.",
                        address
                    );
                    Ok(mis)
                }
                Err(e) => {
                    error!(
                        "Failed to deserialize REMOTE MasterIndexStorage from {}: {}",
                        address, e
                    );
                    // Wrap the serde error
                    Err(Error::Cbor(e))
                }
            }
        }
        Err(Error::KeyNotFound(_)) => {
            debug!("REMOTE Master Index scratchpad not found at {}.", address);
            Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()))
        }
        Err(Error::AutonomiClient(se)) => {
            error!(
                "Autonomi Error fetching REMOTE Master Index {}: {}",
                address, se
            );
            if se.to_string().contains("RecordNotFound") {
                Err(Error::KeyNotFound(MASTER_INDEX_KEY.to_string()))
            } else {
                Err(Error::AutonomiClient(se))
            }
        }
        Err(e) => {
            error!("Error fetching REMOTE Master Index {}: {}", address, e);
            Err(e)
        }
    }
}
