use crate::index::error::IndexError;
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::{StorageError, StorageManager};
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};

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

/// Loads the serialized index data from its dedicated scratchpad.
pub(crate) async fn load_index(
    storage_manager: &dyn StorageManager,
    network_adapter: &dyn NetworkAdapter,
    address: &ScratchpadAddress,
    key: &SecretKey,
) -> Result<MasterIndex, IndexError> {
    // --- Add existence check first ---
    debug!("Checking existence of index scratchpad at {}", address);
    match network_adapter.check_existence(address).await {
        Ok(true) => {
            debug!(
                "Index scratchpad exists at {}. Proceeding to load.",
                address
            );
            // Continue below
        }
        Ok(false) => {
            info!("Index scratchpad not found at address {}.", address);
            // Use DeserializationError to indicate not found after check
            return Err(IndexError::DeserializationError(
                "Master index scratchpad not found on network".to_string(),
            ));
        }
        Err(e) => {
            error!(
                "Error checking existence for index scratchpad {}: {}",
                address, e
            );
            return Err(IndexError::Storage(StorageError::Network(e)));
        }
    }
    // --- End existence check ---

    debug!(
        "Attempting to load MasterIndex data from address: {}",
        address
    );
    // Call read_pad_scratchpad and get Scratchpad object
    match storage_manager.read_pad_scratchpad(address).await {
        Ok(scratchpad) => {
            // Decrypt the data using the provided key
            let decrypted_data = match scratchpad.decrypt_data(key) {
                Ok(data) => data.to_vec(),
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
            // Deserialize the DECRYPTED data
            deserialize_index(&decrypted_data)
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
    network_adapter: &dyn NetworkAdapter, // Added: Needed for existence check
    address: &ScratchpadAddress,
    key: &SecretKey,
    index: &MasterIndex,
) -> Result<(), IndexError> {
    trace!("Persistence: Saving MasterIndex to address {}", address);
    // Serialize the index
    let serialized_data = serialize_index(index)?;

    // Determine if we need to create or update based on existence
    let status_hint = match network_adapter.check_existence(address).await {
        Ok(true) => {
            debug!(
                "Index pad {} exists, using update strategy (Allocated)",
                address
            );
            PadStatus::Allocated // Or Written, assuming update semantics are desired
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
            return Err(IndexError::Storage(StorageError::Network(e)));
        }
    };

    // Use StorageManager to write the data with the determined status
    storage_manager
        .write_pad_data(key, &serialized_data, &status_hint)
        .await
        .map_err(|e| {
            error!("Failed to write index via StorageManager: {}", e);
            IndexError::IndexPersistenceError(format!("Failed to write index to storage: {}", e))
        })?;
    debug!("Persistence: MasterIndex saved successfully.");
    Ok(())
}
