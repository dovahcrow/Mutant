use crate::api::init::initialize_layers;
use crate::api::ReserveCallback;
use crate::data::manager::DataManager;
use crate::error::Error;
use crate::index::manager::IndexManager;
use crate::index::structure::MasterIndex;
use crate::network::{NetworkAdapter, NetworkChoice};
use crate::pad_lifecycle::PadLifecycleManager;
use crate::types::MutAntConfig;
use crate::{GetCallback, InitCallback, KeyDetails, PurgeCallback, PutCallback, StorageStats};
use autonomi::{ScratchpadAddress, SecretKey};
use log::{debug, info, warn};
use std::sync::Arc;

#[derive(Clone)]
pub struct MutAnt {
    data_manager: Arc<dyn DataManager>,
    pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
    index_manager: Arc<dyn IndexManager>,
    network_adapter: Arc<dyn NetworkAdapter>,

    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey,
}

impl MutAnt {
    pub async fn init(private_key_hex: String) -> Result<Self, Error> {
        Self::init_with_progress(private_key_hex, MutAntConfig::default(), None).await
    }

    pub async fn init_with_progress(
        private_key_hex: String,
        config: MutAntConfig,
        init_callback: Option<InitCallback>,
    ) -> Result<Self, Error> {
        info!("MutAnt::init_with_progress started.");
        let (
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address,
            master_index_key,
        ) = initialize_layers(&private_key_hex, &config, init_callback).await?;

        Ok(Self {
            data_manager,
            pad_lifecycle_manager,
            index_manager,
            network_adapter,
            master_index_address,
            master_index_key,
        })
    }

    pub async fn store(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::store called for key '{}'", user_key);
        self.data_manager
            .store(user_key, data_bytes, None)
            .await
            .map_err(Error::Data)
    }

    pub async fn store_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!(
            "MutAnt::store_with_progress called for key \'{}\'",
            user_key
        );
        self.data_manager
            .store(user_key, data_bytes, callback)
            .await
            .map_err(Error::Data)?;

        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after store operation: {}", e);
        }

        Ok(())
    }

    pub async fn fetch(&self, user_key: &str) -> Result<Vec<u8>, Error> {
        debug!("MutAnt::fetch called for key '{}'", user_key);
        self.data_manager
            .fetch(user_key, None)
            .await
            .map_err(Error::Data)
    }

    pub async fn fetch_with_progress(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        debug!("MutAnt::fetch_with_progress called for key '{}'", user_key);
        self.data_manager
            .fetch(user_key, callback)
            .await
            .map_err(Error::Data)
    }

    pub async fn remove(&self, user_key: &str) -> Result<(), Error> {
        debug!("MutAnt::remove called for key '{}'", user_key);
        self.data_manager
            .remove(user_key)
            .await
            .map_err(Error::Data)?;

        if let Err(e) = self.save_index_cache().await {
            warn!("Failed to save index cache after remove operation: {}", e);
        }

        Ok(())
    }

    pub async fn update(&self, user_key: String, data_bytes: &[u8]) -> Result<(), Error> {
        debug!("MutAnt::update called for key '{}'", user_key);
        self.data_manager
            .update(user_key, data_bytes, None)
            .await
            .map_err(Error::Data)
    }

    pub async fn update_with_progress(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), Error> {
        debug!("MutAnt::update_with_progress called for key '{}'", user_key);
        self.data_manager
            .update(user_key, data_bytes, callback)
            .await
            .map_err(Error::Data)
    }

    pub async fn list_keys(&self) -> Result<Vec<String>, Error> {
        debug!("MutAnt::list_keys called");
        self.index_manager.list_keys().await.map_err(Error::Index)
    }

    pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error> {
        debug!("MutAnt::list_key_details called");
        self.index_manager
            .list_all_key_details()
            .await
            .map_err(Error::Index)
    }

    pub async fn get_storage_stats(&self) -> Result<StorageStats, Error> {
        debug!("MutAnt::get_storage_stats called");
        self.index_manager
            .get_storage_stats()
            .await
            .map_err(Error::Index)
    }

    pub async fn import_free_pad(&self, private_key_hex: &str) -> Result<(), Error> {
        debug!("MutAnt::import_free_pad called");
        self.pad_lifecycle_manager
            .import_external_pad(private_key_hex)
            .await
            .map_err(Error::PadLifecycle)
    }

    pub async fn reserve_pads(
        &self,
        count: usize,
        callback: Option<ReserveCallback>,
    ) -> Result<usize, Error> {
        debug!("MutAnt::reserve_pads called, delegating to PadLifecycleManager");
        self.pad_lifecycle_manager
            .reserve_pads(count, callback)
            .await
            .map_err(Error::PadLifecycle)
    }

    pub async fn purge(&self, callback: Option<PurgeCallback>) -> Result<(), Error> {
        debug!("MutAnt::purge called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .purge(callback, network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    pub async fn save_master_index(&self) -> Result<(), Error> {
        debug!("MutAnt::save_master_index called");
        self.index_manager
            .save(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    pub async fn reset(&self) -> Result<(), Error> {
        debug!("MutAnt::reset called");
        self.index_manager
            .reset(&self.master_index_address, &self.master_index_key)
            .await
            .map_err(Error::Index)
    }

    pub async fn save_index_cache(&self) -> Result<(), Error> {
        debug!("MutAnt::save_index_cache called");
        let network_choice = self.network_adapter.get_network_choice();
        self.pad_lifecycle_manager
            .save_index_cache(network_choice)
            .await
            .map_err(Error::PadLifecycle)
    }

    pub async fn fetch_remote_master_index(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::fetch_remote_master_index called");

        self.index_manager
            .fetch_remote(&self.master_index_address)
            .await
            .map_err(Error::Index)
    }

    pub async fn update_internal_master_index(&self, new_index: MasterIndex) -> Result<(), Error> {
        debug!("MutAnt::update_internal_master_index called");
        self.index_manager
            .update_index(new_index)
            .await
            .map_err(Error::Index)
    }

    pub async fn get_index_copy(&self) -> Result<MasterIndex, Error> {
        debug!("MutAnt::get_index_copy called");
        self.index_manager
            .get_index_copy()
            .await
            .map_err(Error::Index)
    }

    pub fn get_network_choice(&self) -> NetworkChoice {
        self.network_adapter.get_network_choice()
    }
}
