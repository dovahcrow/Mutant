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
    network_adapter: &dyn NetworkAdapter, // Network adapter needed for existence check
    address: &ScratchpadAddress,
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
        // Get raw bytes from scratchpad.encrypted_data()
        Ok(scratchpad) => {
            let data = scratchpad.encrypted_data().to_vec();
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
    trace!("Persistence: Saving MasterIndex to address {}", address);
    // Serialize the index
    let serialized_data = serialize_index(index)?;

    // Use StorageManager to write the data
    // We expect the index pad to exist, so pass Allocated status
    storage_manager
        .write_pad_data(key, &serialized_data, &PadStatus::Allocated)
        .await
        .map_err(|e| {
            error!("Failed to write index via StorageManager: {}", e);
            IndexError::IndexPersistenceError(format!("Failed to write index to storage: {}", e))
        })?;
    debug!("Persistence: MasterIndex saved successfully.");
    Ok(())
}
