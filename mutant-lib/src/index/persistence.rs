use crate::index::error::IndexError;
use crate::index::structure::MasterIndex;
use crate::network::NetworkAdapter;
use crate::storage::{StorageError, StorageManager};
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, trace, warn};
use serde_cbor;

/// Serializes the MasterIndex structure into CBOR bytes.
pub(crate) fn serialize_index(index: &MasterIndex) -> Result<Vec<u8>, IndexError> {
    trace!("Serializing MasterIndex");
    serde_cbor::to_vec(index).map_err(|e| {
        error!("CBOR serialization failed: {}", e);
        IndexError::SerializationError(e.to_string())
    })
}

/// Deserializes CBOR bytes into a MasterIndex structure.
pub(crate) fn deserialize_index(data: &[u8]) -> Result<MasterIndex, IndexError> {
    trace!("Deserializing MasterIndex from {} bytes", data.len());
    serde_cbor::from_slice(data).map_err(|e| {
        error!("CBOR deserialization failed: {}", e);
        IndexError::from(e) // Use the From impl in error.rs
    })
}

/// Loads the serialized index data from its dedicated scratchpad using the StorageManager.
/// First checks if the scratchpad exists.
pub(crate) async fn load_index(
    storage_manager: &dyn StorageManager,
    network_adapter: &dyn NetworkAdapter,
    address: &ScratchpadAddress,
) -> Result<MasterIndex, IndexError> {
    debug!("Checking existence of MasterIndex at address: {}", address);

    // --- Check existence first --- (THIS IS NEW)
    match network_adapter.check_existence(address).await {
        Ok(true) => {
            debug!("Index scratchpad found at {}, proceeding to load.", address);
            // Continue to load data
        }
        Ok(false) => {
            debug!("Index scratchpad not found at address {}.", address);
            return Err(IndexError::KeyNotFound(format!(
                "Master index record not found at address {}",
                address
            )));
        }
        Err(e) => {
            error!(
                "Error checking existence for index scratchpad {}: {}",
                address, e
            );
            // Propagate the error, mapping it appropriately
            return Err(IndexError::Storage(StorageError::Network(e)));
        }
    }
    // --- End existence check ---

    debug!(
        "Attempting to load MasterIndex data from address: {}",
        address
    );
    match storage_manager.read_pad_data(address).await {
        Ok(data) => {
            if data.is_empty() {
                warn!(
                    "Loaded empty data from existing index address {}, treating as invalid.",
                    address
                );
                return Err(IndexError::DeserializationError(
                    "Index data read from storage was empty".to_string(),
                ));
            }
            debug!(
                "Successfully read {} bytes from index address {}",
                data.len(),
                address
            );
            deserialize_index(&data)
        }
        Err(e) => {
            // Removed the old RecordNotFound check as existence is checked upfront
            error!("Storage error during index load from {}: {}", address, e);
            Err(IndexError::Storage(e))
        }
    }
}

/// Saves the serialized index data to its dedicated scratchpad using the StorageManager.
pub(crate) async fn save_index(
    storage_manager: &dyn StorageManager,
    address: &ScratchpadAddress,
    key: &SecretKey,
    index: &MasterIndex,
) -> Result<(), IndexError> {
    debug!("Attempting to save MasterIndex to address: {}", address);
    let serialized_data = serialize_index(index)?;
    debug!("Serialized index size: {} bytes", serialized_data.len());
    let written_address = storage_manager
        .write_pad_data(key, &serialized_data)
        .await
        .map_err(|e| {
            error!("Storage error during index save to {}: {}", address, e);
            IndexError::Storage(e)
        })?;
    if written_address != *address {
        warn!(
            "Index save address mismatch: Expected {}, Got {}. This might indicate an issue.",
            address, written_address
        );
        // Continue anyway, assuming the data is written correctly to the key's derived address.
    }
    debug!(
        "Successfully saved MasterIndex (Address: {})",
        written_address
    );
    Ok(())
}
