use crate::index::structure::PadStatus;
use crate::network::NetworkAdapter;
use crate::storage::error::StorageError;
use async_trait::async_trait;
use autonomi::{Scratchpad, ScratchpadAddress, SecretKey};
use log::trace;
use std::sync::Arc;

/// Trait defining the interface for storing and retrieving data from individual scratchpads.
/// Handles potential data transformations (encryption, checksums - if implemented).
#[async_trait]
pub trait StorageManager: Send + Sync {
    /// Reads the full Scratchpad object from a specific scratchpad address.
    async fn read_pad_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, StorageError>;

    /// Writes potentially transformed data to a scratchpad using its associated secret key.
    /// The `is_new` hint tells the underlying network layer if it can skip existence checks.
    /// Returns the address of the written pad.
    async fn write_pad_data(
        &self,
        key: &SecretKey,
        data: &[u8],
        current_status: &PadStatus,
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
    async fn read_pad_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, StorageError> {
        trace!(
            "StorageManager::read_pad_scratchpad for address: {}",
            address
        );
        // Read the full scratchpad object. No decryption/transformation at this layer.
        let scratchpad = self.network_adapter.get_raw_scratchpad(address).await?;
        // Apply transformations (e.g., checksum verification if added) from pad_io if needed
        Ok(scratchpad)
    }

    async fn write_pad_data(
        &self,
        key: &SecretKey,
        data: &[u8],
        current_status: &PadStatus,
    ) -> Result<ScratchpadAddress, StorageError> {
        trace!(
            "StorageManager::write_pad_data, data_len: {}, status: {:?}",
            data.len(),
            current_status
        );
        // TODO: Add encryption/checksum generation here if implemented in pad_io.rs
        let transformed_data = data; // Placeholder

        // Call network adapter's put_raw with the key and status
        let address = self
            .network_adapter
            .put_raw(key, transformed_data, current_status)
            .await?;
        Ok(address)
    }
}
