use crate::config::NetworkChoice;
use crate::index::pad_info::PadInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Re-export modules
mod core;
mod key_management;
mod pad_management;
mod status;
mod public_keys;
mod import_export;
mod utils;

#[cfg(test)]
mod tests;

// Re-export utility functions
pub use utils::get_index_file_path;

/// Represents an entry in the master index, which can be either private key data or public upload data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum IndexEntry {
    PrivateKey(Vec<PadInfo>),
    PublicUpload(PadInfo, Vec<PadInfo>),
}

/// The central index managing all keys and scratchpads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MasterIndex {
    /// Mapping from key names (e.g., file paths or public upload IDs) to their detailed information.
    index: BTreeMap<String, IndexEntry>,

    /// List of scratchpads that are currently free and available for allocation.
    /// Each tuple contains the address, the associated encryption key, and the generation ID.
    free_pads: Vec<PadInfo>,

    /// List of scratchpads that are awaiting verification.
    /// Each tuple contains the address and the associated encryption key.
    pending_verification_pads: Vec<PadInfo>,

    network_choice: NetworkChoice,
}

#[derive(Debug, Default)]
pub struct StorageStats {
    pub nb_keys: u64,
    pub total_pads: u64,
    pub occupied_pads: u64,
    pub free_pads: u64,
    pub pending_verification_pads: u64,
}
