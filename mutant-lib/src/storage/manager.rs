use crate::index::structure::PadStatus;
use crate::network::AutonomiNetworkAdapter;
use crate::storage::error::StorageError;

use autonomi::{Scratchpad, ScratchpadAddress, SecretKey};
use log::trace;
use std::sync::Arc;

pub struct DefaultStorageManager {
    network_adapter: Arc<AutonomiNetworkAdapter>,
}

impl DefaultStorageManager {
    pub fn new(network_adapter: Arc<AutonomiNetworkAdapter>) -> Self {
        Self { network_adapter }
    }

    pub async fn read_pad_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, StorageError> {
        trace!(
            "StorageManager::read_pad_scratchpad for address: {}",
            address
        );

        let scratchpad = self.network_adapter.get_raw_scratchpad(address).await?;

        Ok(scratchpad)
    }

    pub async fn write_pad_data(
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

        let transformed_data = data;

        let address = self
            .network_adapter
            .put_raw(key, transformed_data, current_status)
            .await?;
        Ok(address)
    }
}
