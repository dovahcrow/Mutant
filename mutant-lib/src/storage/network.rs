use super::ContentType;
use crate::error::Error;
use crate::mutant::data_structures::MasterIndexStorage;
use crate::mutant::MASTER_INDEX_KEY;
use crate::utils::retry::retry_operation;
use autonomi::{client::payment::PaymentOption, Bytes, Client, ScratchpadAddress, SecretKey};
use hex;
use log::{debug, error, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task;

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
        |e: &Error| !matches!(e, Error::KeyNotFound(_)),
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
    retry_operation(
        &format!("Update scratchpad owned by {:?}", owner_pubkey),
        || async {
            client
                .scratchpad_update(owner_key, content_type, &data_bytes)
                .await
                .map_err(Error::AutonomiClient)
        },
        |_e: &Error| true,
    )
    .await
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

    debug!(
        "Attempting static scratchpad creation for owner {:?} at expected address {}",
        owner_key.public_key(),
        expected_address
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
        "Static scratchpad creation successful at {}",
        created_address
    );
    Ok(created_address)
}
