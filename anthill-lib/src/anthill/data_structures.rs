use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents how and where a logical piece of data (identified by user key) is stored.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyStorageInfo {
    /// List of pads holding the data for this key (Address, Key Bytes)
    pub(crate) pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    /// Actual size of the data stored across these pads
    pub(crate) data_size: usize,
    /// Timestamp of the last modification (store or update)
    #[serde(default = "Utc::now")]
    pub(crate) modified: DateTime<Utc>,
}

/// The main index mapping user keys to their storage information.
pub type MasterIndex = HashMap<String, KeyStorageInfo>;

/// Wrapper struct for data stored in the Master Index scratchpad.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MasterIndexStorage {
    /// The actual mapping of user keys to data locations.
    pub(crate) index: MasterIndex,
    /// List of (Address, Key Bytes) tuples for scratchpads available for reuse.
    #[serde(default)]
    pub(crate) free_pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    /// The standard size of scratchpads managed by this index (persisted).
    pub(crate) scratchpad_size: usize,
}
