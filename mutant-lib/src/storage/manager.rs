use crate::network::NetworkAdapter;
use crate::storage::error::StorageError;
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::trace;
use std::sync::Arc;

/// Trait defining the interface for a slightly higher-level storage abstraction
/// over raw scratchpads. It might handle transformations like encryption or checksums.
#[async_trait]
pub trait StorageManager: Send + Sync {
    /// Reads potentially transformed *encrypted* data from a specific scratchpad address.
    async fn read_pad_data(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, StorageError>;

    /// Writes potentially transformed data to a scratchpad using its associated secret key.
    /// Returns the address of the written pad.
    async fn write_pad_data(
        &self,
        key: &SecretKey,
        data: &[u8],
    ) -> Result<ScratchpadAddress, StorageError>;

    // Potential future additions:
    // async fn delete_pad_data(&self, address: &ScratchpadAddress, key: &SecretKey) -> Result<(), StorageError>;
    // fn get_usable_pad_size(&self) -> usize; // If transformations affect usable size
}

// --- Implementation ---

/// Default implementation of StorageManager that acts as a direct pass-through
/// to the underlying NetworkAdapter, without additional transformations initially.
pub struct DefaultStorageManager {
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultStorageManager {
    /// Creates a new DefaultStorageManager.
    pub fn new(network_adapter: Arc<dyn NetworkAdapter>) -> Self {
        Self { network_adapter }
    }
}

#[async_trait]
impl StorageManager for DefaultStorageManager {
    async fn read_pad_data(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, StorageError> {
        trace!("StorageManager::read_pad_data for address: {}", address);
        // Read raw encrypted data. No decryption at this layer.
        let raw_data = self.network_adapter.get_raw(address).await?;
        // Apply transformations (e.g., checksum verification if added) from pad_io if needed
        Ok(raw_data)
    }

    async fn write_pad_data(
        &self,
        key: &SecretKey,
        data: &[u8],
    ) -> Result<ScratchpadAddress, StorageError> {
        trace!("StorageManager::write_pad_data, data_len: {}", data.len());
        // TODO: Add encryption/checksum generation here if implemented in pad_io.rs
        let transformed_data = data; // Placeholder

        // Call network adapter's put_raw with the key
        let address = self.network_adapter.put_raw(key, transformed_data).await?; // Propagate NetworkError (converted via From)
        Ok(address)
    }
}
