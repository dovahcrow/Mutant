use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::Storage;
use std::sync::Arc;
use tokio::sync::Mutex;

// --- Public Modules ---
pub mod delete;
pub mod read;
pub mod util;
pub mod write;

/// Manages scratchpad allocation, I/O, and recycling.
#[derive(Clone)] // Clone is needed if MutAnt is Clone
pub(crate) struct PadManager {
    storage: Arc<Storage>,
    master_index_storage: Arc<Mutex<MasterIndexStorage>>,
    // Concurrency limits could be added here later if needed
}

impl PadManager {
    /// Creates a new PadManager instance.
    ///
    /// # Arguments
    ///
    /// * `storage` - Shared access to the underlying storage system.
    /// * `master_index_storage` - Shared mutex-protected access to the master index.
    pub(crate) fn new(
        storage: Arc<Storage>,
        master_index_storage: Arc<Mutex<MasterIndexStorage>>,
    ) -> Self {
        PadManager {
            storage,
            master_index_storage,
        }
    }
}
