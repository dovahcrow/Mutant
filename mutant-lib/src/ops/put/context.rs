use crate::index::{PadInfo, PadStatus};
use crate::network::Network;
use std::{ops::Range, sync::Arc};
use tokio::sync::RwLock;
use mutant_protocol::PutCallback;

/// Context for put operations
#[derive(Clone)]
pub struct Context {
    pub index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    pub network: Arc<Network>,
    pub name: Arc<String>,
    pub data: Arc<Vec<u8>>,
    pub chunk_ranges: Arc<Vec<Range<usize>>>,
    pub public: bool,
    /// Optional preserved index pad for public key updates
    pub preserved_index_pad: Option<PadInfo>,
    /// Flag to indicate if this context is for an index pad
    pub is_index_pad: bool,
}

/// Context for put tasks
pub struct PutTaskContext {
    pub base_context: Context,
    pub no_verify: Arc<bool>,
    pub put_callback: Option<PutCallback>,
}
