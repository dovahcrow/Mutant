use crate::index::PadInfo;
use autonomi::SecretKey;
use std::sync::Arc;
use tokio::time::Duration;

pub(crate) const CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(1);
pub(crate) const CONFIRMATION_RETRY_LIMIT: u32 = 360;

#[derive(Clone)]
pub(crate) struct DataManagerDependencies {
    pub index_manager: Arc<dyn crate::index::IndexManager>,
    pub pad_lifecycle_manager: Arc<dyn crate::pad_lifecycle::PadLifecycleManager>,
    pub storage_manager: Arc<dyn crate::storage::StorageManager>,

    pub network_adapter: Arc<dyn crate::network::NetworkAdapter>,
}

#[derive(Clone)]
pub(crate) struct WriteTaskInput {
    pub(crate) pad_info: PadInfo,
    pub(crate) secret_key: SecretKey,
    pub(crate) chunk_data: Vec<u8>,
}
