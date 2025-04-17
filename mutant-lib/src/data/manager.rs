use crate::data::error::DataError;
use crate::data::ops;
use crate::data::ops::common::DataManagerDependencies;
use crate::events::{GetCallback, PutCallback};
use crate::index::IndexManager;
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::PadLifecycleManager;
use crate::storage::StorageManager;
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait DataManager: Send + Sync {
    async fn store(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), DataError>;

    async fn fetch(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, DataError>;

    async fn remove(&self, user_key: &str) -> Result<(), DataError>;

    async fn update(
        &self,
        _user_key: String,
        _data_bytes: &[u8],
        _callback: Option<PutCallback>,
    ) -> Result<(), DataError>;
}

pub struct DefaultDataManager {
    deps: DataManagerDependencies,
}

impl DefaultDataManager {
    pub fn new(
        index_manager: Arc<dyn IndexManager>,
        pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
        storage_manager: Arc<dyn StorageManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            deps: DataManagerDependencies {
                index_manager,
                pad_lifecycle_manager,
                storage_manager,
                network_adapter,
            },
        }
    }
}

#[async_trait]
impl DataManager for DefaultDataManager {
    async fn store(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), DataError> {
        ops::store::store_op(&self.deps, user_key, data_bytes, callback).await
    }

    async fn fetch(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, DataError> {
        ops::fetch::fetch_op(&self.deps, user_key, callback).await
    }

    async fn remove(&self, user_key: &str) -> Result<(), DataError> {
        ops::remove::remove_op(&self.deps, user_key).await
    }

    async fn update(
        &self,
        _user_key: String,
        _data_bytes: &[u8],
        _callback: Option<PutCallback>,
    ) -> Result<(), DataError> {
        unimplemented!(
            "Update operation is temporarily removed due to complexity. Use remove then store."
        );
    }
}
