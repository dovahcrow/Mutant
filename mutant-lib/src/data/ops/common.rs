// Common definitions for data operations
use crate::index::PadInfo;
use autonomi::SecretKey;
use std::sync::Arc;
use tokio::time::Duration;

// --- Constants ---
pub(crate) const CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(1);
pub(crate) const CONFIRMATION_RETRY_LIMIT: u32 = 360;

// Helper structure to pass down dependencies to operation functions
// Using Arcs for shared ownership across potential concurrent tasks
#[derive(Clone)] // Add Clone trait
pub(crate) struct DataManagerDependencies {
    pub index_manager: Arc<dyn crate::index::IndexManager>,
    pub pad_lifecycle_manager: Arc<dyn crate::pad_lifecycle::PadLifecycleManager>,
    pub storage_manager: Arc<dyn crate::storage::StorageManager>,
    // Add network_adapter if needed for existence checks? Yes.
    pub network_adapter: Arc<dyn crate::network::NetworkAdapter>,
}

/// Structure to hold the necessary information for a single write task.
#[derive(Clone)] // Clone needed if the preparation function returns owned values
pub(crate) struct WriteTaskInput {
    pub(crate) pad_info: PadInfo,
    pub(crate) secret_key: SecretKey,
    pub(crate) chunk_data: Vec<u8>,
}
