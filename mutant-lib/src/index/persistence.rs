use crate::index::error::IndexError;
use crate::index::structure::MasterIndex;
use crate::storage::{StorageError, StorageManager};
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, trace};
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
pub(crate) async fn load_index(
    storage_manager: &dyn StorageManager,
    address: &ScratchpadAddress,
) -> Result<MasterIndex, IndexError> {
    debug!("Attempting to load MasterIndex from address: {}", address);
    match storage_manager.read_pad_data(address).await {
        Ok(data) => {
            if data.is_empty() {
                // Treat empty data as not found, as CBOR deserialization might succeed on empty slice
                debug!(
                    "Loaded empty data from index address {}, treating as not found.",
                    address
                );
                // TODO: Define a specific error for this? Or rely on DeserializationError::EOF?
                // For now, let deserialize_index handle it, which should map EOF.
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
        Err(StorageError::Network(net_err)) => {
            // Check if the network error indicates the record wasn't found
            // This relies on the specific error message from autonomi::ClientError
            // TODO: Make this check more robust if possible (e.g., specific error variant)
            if net_err.to_string().contains("RecordNotFound")
                || net_err.to_string().contains("Could not find record")
            {
                debug!(
                    "MasterIndex not found at address {} (RecordNotFound)",
                    address
                );
                Err(IndexError::KeyNotFound(format!(
                    "Master index record not found at address {}",
                    address
                )))
            } else {
                error!(
                    "Network error during index load from {}: {}",
                    address, net_err
                );
                Err(IndexError::Storage(StorageError::Network(net_err)))
            }
        }
        Err(e) => {
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
    storage_manager
        .write_pad_data(address, &serialized_data)
        .await
        .map_err(|e| {
            error!("Storage error during index save to {}: {}", address, e);
            IndexError::Storage(e)
        })?;
    debug!("Successfully saved MasterIndex to address {}", address);
    Ok(())
}
