use crate::index::error::IndexError;
use crate::index::structure::{MasterIndex, PadStatus};
use crate::network::AutonomiNetworkAdapter;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};

/// Serializes the `MasterIndex` into CBOR format.
///
/// # Arguments
///
/// * `index` - A reference to the `MasterIndex` to serialize.
///
/// # Errors
///
/// Returns `IndexError::SerializationError` if CBOR serialization fails.
pub(crate) fn serialize_index(index: &MasterIndex) -> Result<Vec<u8>, IndexError> {
    trace!("Serializing MasterIndex");
    serde_cbor::to_vec(index).map_err(|e| {
        error!("CBOR serialization failed: {}", e);
        IndexError::SerializationError(e.to_string())
    })
}

/// Deserializes a `MasterIndex` from CBOR-encoded bytes.
///
/// # Arguments
///
/// * `data` - The byte slice containing the CBOR-encoded index data.
///
/// # Errors
///
/// Returns `IndexError::DeserializationError` if CBOR deserialization fails.
pub(crate) fn deserialize_index(data: &[u8]) -> Result<MasterIndex, IndexError> {
    trace!("Deserializing MasterIndex from {} bytes", data.len());
    serde_cbor::from_slice(data).map_err(|e| {
        error!("CBOR deserialization failed: {}", e);
        IndexError::from(e)
    })
}

/// Loads the `MasterIndex` from a specific scratchpad address on the network.
///
/// Checks for the existence of the scratchpad first. If found, it fetches,
/// decrypts, and deserializes the index data.
///
/// # Arguments
///
/// * `network_adapter` - The network adapter to use for fetching data.
/// * `address` - The address of the scratchpad containing the index.
/// * `key` - The secret key required to decrypt the index data.
///
/// # Errors
///
/// Returns `IndexError` if:
/// - Checking scratchpad existence fails (`IndexError::Network`).
/// - The scratchpad does not exist (`IndexError::DeserializationError`).
/// - Fetching the scratchpad fails (`IndexError::Network`).
/// - Decrypting the index data fails (`IndexError::DecryptionError`).
/// - The decrypted data is empty (`IndexError::DeserializationError`).
/// - Deserializing the index data fails (`IndexError::DeserializationError`).
pub(crate) async fn load_index(
    network_adapter: &AutonomiNetworkAdapter,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<MasterIndex, IndexError> {
    debug!("Checking existence of index scratchpad at {}", address);
    match network_adapter.check_existence(address).await {
        Ok(true) => {
            debug!(
                "Index scratchpad exists at {}. Proceeding to load.",
                address
            );
        }
        Ok(false) => {
            info!("Index scratchpad not found at address {}.", address);

            return Err(IndexError::DeserializationError(
                "Master index scratchpad not found on network".to_string(),
            ));
        }
        Err(e) => {
            error!(
                "Error checking existence for index scratchpad {}: {}",
                address, e
            );
            return Err(IndexError::Network(e));
        }
    }

    debug!(
        "Attempting to load MasterIndex data from address: {}",
        address
    );

    match network_adapter.get_raw_scratchpad(address).await {
        Ok(scratchpad) => {
            let decrypted_data = match scratchpad.decrypt_data(key) {
                Ok(data_bytes) => data_bytes.to_vec(),
                Err(e) => {
                    error!("Failed to decrypt index data from {}: {}", address, e);
                    return Err(IndexError::DecryptionError(e.to_string()));
                }
            };

            if decrypted_data.is_empty() {
                warn!(
                    "Loaded and decrypted empty data from index address {}, treating as invalid.",
                    address
                );
                return Err(IndexError::DeserializationError(
                    "Index data read and decrypted from storage was empty".to_string(),
                ));
            }
            debug!(
                "Successfully read and decrypted {} bytes from index address {}",
                decrypted_data.len(),
                address
            );

            deserialize_index(&decrypted_data)
        }
        Err(e) => {
            error!("Network error during index load from {}: {}", address, e);
            Err(IndexError::Network(e))
        }
    }
}

/// Saves the `MasterIndex` to a specific scratchpad address on the network.
///
/// Serializes the index, checks if the target scratchpad exists to determine
/// whether to create or update, and then uses the network adapter to write the data.
///
/// # Arguments
///
/// * `network_adapter` - The network adapter to use for writing data.
/// * `address` - The address of the scratchpad where the index should be saved.
/// * `key` - The secret key used to encrypt the index data.
/// * `index` - A reference to the `MasterIndex` to save.
///
/// # Errors
///
/// Returns `IndexError` if:
/// - Serialization fails (`IndexError::SerializationError`).
/// - Checking scratchpad existence fails (`IndexError::Network`).
/// - Writing the data via the network adapter fails (`IndexError::IndexPersistenceError`).
pub(crate) async fn save_index(
    network_adapter: &AutonomiNetworkAdapter,
    address: &ScratchpadAddress,
    key: &SecretKey,
    index: &MasterIndex,
) -> Result<(), IndexError> {
    trace!("Persistence: Saving MasterIndex to address {}", address);

    let serialized_data = serialize_index(index)?;

    let status_hint = match network_adapter.check_existence(address).await {
        Ok(true) => {
            debug!(
                "Index pad {} exists, using update strategy (Allocated)",
                address
            );
            PadStatus::Allocated
        }
        Ok(false) => {
            debug!(
                "Index pad {} does not exist, using create strategy (Generated)",
                address
            );
            PadStatus::Generated
        }
        Err(e) => {
            error!(
                "Failed to check existence for index pad {} before save: {}",
                address, e
            );
            return Err(IndexError::Network(e));
        }
    };

    network_adapter
        .put_raw(key, &serialized_data, &status_hint)
        .await
        .map_err(|e| {
            error!("Failed to write index via NetworkAdapter: {}", e);
            IndexError::IndexPersistenceError(format!("Failed to write index to network: {}", e))
        })?;
    debug!("Persistence: MasterIndex saved successfully.");
    Ok(())
}
