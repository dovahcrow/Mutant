use crate::index::manager::DefaultIndexManager;
use crate::index::PadInfo;
use crate::network::AutonomiNetworkAdapter;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use autonomi::SecretKey;
use std::sync::Arc;
use tokio::time::Duration;

pub(crate) const CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(1);
pub(crate) const CONFIRMATION_RETRY_LIMIT: u32 = 360;

#[derive(Clone)]
pub(crate) struct DataManagerDependencies {
    pub index_manager: Arc<DefaultIndexManager>,
    pub pad_lifecycle_manager: Arc<DefaultPadLifecycleManager>,
    pub network_adapter: Arc<AutonomiNetworkAdapter>,
}

#[derive(Clone)]
pub(crate) struct WriteTaskInput {
    pub(crate) pad_info: PadInfo,
    pub(crate) secret_key: SecretKey,
    pub(crate) chunk_data: Vec<u8>,
}
