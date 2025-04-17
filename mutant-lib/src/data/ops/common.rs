// Common definitions for data operations
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
