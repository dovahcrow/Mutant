use crate::index::error::IndexError;
use crate::index::structure::{MasterIndex, PadStatus};
use crate::network::AutonomiNetworkAdapter;
use crate::storage::error::StorageError;
use crate::storage::manager::DefaultStorageManager;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};

pub(crate) fn serialize_index(index: &MasterIndex) -> Result<Vec<u8>, IndexError> {
    trace!("Serializing MasterIndex");
    serde_cbor::to_vec(index).map_err(|e| {
        error!("CBOR serialization failed: {}", e);
        IndexError::SerializationError(e.to_string())
    })
}

pub(crate) fn deserialize_index(data: &[u8]) -> Result<MasterIndex, IndexError> {
    trace!("Deserializing MasterIndex from {} bytes", data.len());
    serde_cbor::from_slice(data).map_err(|e| {
        error!("CBOR deserialization failed: {}", e);
        IndexError::from(e)
    })
}

pub(crate) async fn load_index(
    storage_manager: &DefaultStorageManager,
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
            return Err(IndexError::Storage(StorageError::Network(e)));
        }
    }

    debug!(
        "Attempting to load MasterIndex data from address: {}",
        address
    );

    match storage_manager.read_pad_scratchpad(address).await {
        Ok(scratchpad) => {
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

            deserialize_index(&decrypted_data)
        }
        Err(e) => {
            error!("Storage error during index load from {}: {}", address, e);
            Err(IndexError::Storage(e))
        }
    }
}

pub(crate) async fn save_index(
    storage_manager: &DefaultStorageManager,
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
            return Err(IndexError::Storage(StorageError::Network(e)));
        }
    };

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
