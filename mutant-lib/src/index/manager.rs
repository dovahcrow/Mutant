use crate::index::error::IndexError;
use crate::index::persistence::{load_index, save_index};
use crate::index::query;
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::NetworkAdapter;
use crate::storage::StorageManager;
use crate::types::{KeyDetails, StorageStats};
use async_trait::async_trait;
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait IndexManager: Send + Sync {
    async fn load_or_initialize(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError>;

    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError>;

    async fn list_keys(&self) -> Result<Vec<String>, IndexError>;

    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError>;

    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError>;

    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError>;

    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError>;

    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;

    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError>;

    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError>;

    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError>;

    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError>;

    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError>;

    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError>;

    async fn get_index_copy(&self) -> Result<MasterIndex, IndexError>;

    async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError>;

    async fn get_scratchpad_size(&self) -> Result<usize, IndexError>;

    async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
    ) -> Result<MasterIndex, IndexError>;

    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError>;
}

pub struct DefaultIndexManager {
    state: Mutex<MasterIndex>,
    storage_manager: Arc<dyn StorageManager>,
    network_adapter: Arc<dyn NetworkAdapter>,
}

impl DefaultIndexManager {
    pub fn new(
        storage_manager: Arc<dyn StorageManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            state: Mutex::new(MasterIndex::default()),
            storage_manager,
            network_adapter,
        }
    }
}

#[async_trait]
impl IndexManager for DefaultIndexManager {
    async fn load_or_initialize(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Loading index from storage...");
        match load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            master_index_key,
        )
        .await
        {
            Ok(loaded_index) => {
                let mut state_guard = self.state.lock().await;

                if loaded_index.scratchpad_size == 0 {
                    warn!(
                        "Loaded index has scratchpad_size 0, setting to default: {}",
                        DEFAULT_SCRATCHPAD_SIZE
                    );
                    *state_guard = MasterIndex {
                        scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
                        ..loaded_index
                    };
                } else {
                    *state_guard = loaded_index;
                }
                info!("Index loaded successfully from storage.");
                Ok(())
            }

            Err(IndexError::DeserializationError(e)) => {
                warn!(
                    "Failed to load or deserialize index from storage ({}). Initializing with default in memory.",
                    e
                );

                Ok(())
            }

            Err(IndexError::KeyNotFound(_)) => {
                warn!("Index key not found (unexpected persistence state?). Initializing with default in memory.");
                Ok(())
            }

            Err(e) => {
                error!("Failed to load index from storage: {}", e);
                Err(e)
            }
        }
    }

    async fn save(
        &self,
        master_index_address: &ScratchpadAddress,
        _master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        debug!("IndexManager: Saving index to storage...");
        let state_guard = self.state.lock().await;
        save_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            _master_index_key,
            &*state_guard,
        )
        .await
    }

    async fn get_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_info_internal(&*state_guard, key).cloned())
    }

    async fn insert_key_info(&self, key: String, info: KeyInfo) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::insert_key_info_internal(&mut *state_guard, key, info)
    }

    async fn remove_key_info(&self, key: &str) -> Result<Option<KeyInfo>, IndexError> {
        let mut state_guard = self.state.lock().await;
        let removed_info = query::remove_key_info_internal(&mut *state_guard, key);

        if let Some(ref info) = removed_info {
            let mut pads_to_verify = Vec::new();

            let mut pads_to_free_directly = Vec::new();

            for pad_info in &info.pads {
                if let Some(pad_key) = info.pad_keys.get(&pad_info.address).cloned() {
                    match pad_info.status {
                        PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                            trace!(
                                "Pad {} for removed key '{}' is {:?}. Adding directly to free pool.",
                                pad_info.address,
                                key,
                                pad_info.status
                            );

                            pads_to_free_directly.push((pad_info.address, pad_key, 0u64));
                        }

                        PadStatus::Generated => {
                            trace!(
                                "Pad {} for removed key '{}' is {:?}. Adding to pending verification.",
                                pad_info.address,
                                key,
                                pad_info.status
                            );
                            pads_to_verify.push((pad_info.address, pad_key));
                        }
                    }
                } else {
                    warn!(
                        "Missing pad key for pad {} while removing key '{}'. Cannot release.",
                        pad_info.address, key
                    );
                }
            }

            if !pads_to_free_directly.is_empty() {
                debug!(
                    "Key '{}': Directly adding {} pads to the free list.",
                    key,
                    pads_to_free_directly.len()
                );

                query::add_free_pads_with_counters_internal(
                    &mut *state_guard,
                    pads_to_free_directly,
                )?;
            }

            if !pads_to_verify.is_empty() {
                debug!(
                    "Key '{}': Adding {} pads to the pending verification list.",
                    key,
                    pads_to_verify.len()
                );

                query::add_pending_verification_pads_internal(&mut *state_guard, pads_to_verify)?;
            }
        }

        Ok(removed_info)
    }

    async fn list_keys(&self) -> Result<Vec<String>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_keys_internal(&*state_guard))
    }

    async fn get_key_details(&self, key: &str) -> Result<Option<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::get_key_details_internal(&*state_guard, key))
    }

    async fn list_all_key_details(&self) -> Result<Vec<KeyDetails>, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(query::list_all_key_details_internal(&*state_guard))
    }

    async fn get_storage_stats(&self) -> Result<StorageStats, IndexError> {
        let state_guard = self.state.lock().await;
        query::get_stats_internal(&*state_guard)
    }

    async fn add_free_pad(
        &self,
        address: ScratchpadAddress,
        key_bytes: Vec<u8>,
    ) -> Result<(), IndexError> {
        match self.storage_manager.read_pad_scratchpad(&address).await {
            Ok(scratchpad) => {
                let counter = scratchpad.counter();
                trace!(
                    "Fetched counter {} for pad {} before adding to free list.",
                    counter,
                    address
                );
                let mut state_guard = self.state.lock().await;
                query::add_free_pad_with_counter_internal(
                    &mut *state_guard,
                    address,
                    key_bytes,
                    counter,
                )
            }
            Err(e) => {
                warn!("Failed to fetch scratchpad to get counter for pad {} during add_free_pad: {}. Pad will not be added to free list.", address, e);

                Ok(())
            }
        }
    }

    async fn add_free_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        let mut pads_with_counters = Vec::with_capacity(pads.len());
        for (address, key_bytes) in pads {
            match self.storage_manager.read_pad_scratchpad(&address).await {
                Ok(scratchpad) => {
                    let counter = scratchpad.counter();
                    trace!(
                        "Fetched counter {} for pad {} before adding to free list.",
                        counter,
                        address
                    );
                    pads_with_counters.push((address, key_bytes, counter));
                }
                Err(e) => {
                    warn!("Failed to fetch scratchpad to get counter for pad {} during add_free_pads: {}. Pad will not be added to free list.", address, e);
                }
            }
        }

        if !pads_with_counters.is_empty() {
            let mut state_guard = self.state.lock().await;

            query::add_free_pads_with_counters_internal(&mut *state_guard, pads_with_counters)?;
        }
        Ok(())
    }

    async fn take_free_pad(&self) -> Result<Option<(ScratchpadAddress, Vec<u8>, u64)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_free_pad_internal(&mut *state_guard))
    }

    async fn take_pending_pads(&self) -> Result<Vec<(ScratchpadAddress, Vec<u8>)>, IndexError> {
        let mut state_guard = self.state.lock().await;
        Ok(query::take_pending_pads_internal(&mut *state_guard))
    }

    async fn remove_from_pending(
        &self,
        address_to_remove: &ScratchpadAddress,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::remove_from_pending_internal(&mut *state_guard, address_to_remove)
    }

    async fn update_pad_status(
        &self,
        key: &str,
        pad_address: &ScratchpadAddress,
        new_status: PadStatus,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::update_pad_status_internal(&mut *state_guard, key, pad_address, new_status)
    }

    async fn mark_key_complete(&self, key: &str) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::mark_key_complete_internal(&mut *state_guard, key)
    }

    async fn reset(
        &self,
        master_index_address: &ScratchpadAddress,
        master_index_key: &SecretKey,
    ) -> Result<(), IndexError> {
        info!("IndexManager: Resetting index...");
        {
            let mut state_guard = self.state.lock().await;
            query::reset_index_internal(&mut *state_guard);
        }

        self.save(master_index_address, master_index_key).await?;
        info!("Index reset and saved successfully.");
        Ok(())
    }

    async fn get_index_copy(&self) -> Result<MasterIndex, IndexError> {
        let state_guard = self.state.lock().await;
        Ok(state_guard.clone())
    }

    async fn update_index(&self, new_index: MasterIndex) -> Result<(), IndexError> {
        info!("IndexManager: Updating in-memory index directly.");
        let mut state_guard = self.state.lock().await;
        *state_guard = new_index;
        Ok(())
    }

    async fn get_scratchpad_size(&self) -> Result<usize, IndexError> {
        let state_guard = self.state.lock().await;
        let size = state_guard.scratchpad_size;
        if size == 0 {
            warn!("Index reports scratchpad size of 0, returning default.");
            Ok(DEFAULT_SCRATCHPAD_SIZE)
        } else {
            Ok(size)
        }
    }

    async fn fetch_remote(
        &self,
        master_index_address: &ScratchpadAddress,
    ) -> Result<MasterIndex, IndexError> {
        debug!("IndexManager: Fetching remote index directly...");

        let dummy_key = SecretKey::random();
        load_index(
            self.storage_manager.as_ref(),
            self.network_adapter.as_ref(),
            master_index_address,
            &dummy_key,
        )
        .await
    }

    async fn add_pending_pads(
        &self,
        pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    ) -> Result<(), IndexError> {
        let mut state_guard = self.state.lock().await;
        query::add_pending_pads_internal(&mut *state_guard, pads)
    }
}
