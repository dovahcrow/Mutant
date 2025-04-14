use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents how and where a logical piece of data (identified by user key) is stored.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PadUploadStatus {
    Generated, // Pad metadata known, initial write+confirm pending
    Free,      // Pad reservation confirmed, data write+confirm pending
    Populated, // Data chunk confirmed written to this pad
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PadInfo {
    pub address: ScratchpadAddress,
    pub key: Vec<u8>,
    pub status: PadUploadStatus,
    pub is_new: bool, // Flag to indicate if this pad needs creation vs update
                      // Optional: pub chunk_index: usize, // If order matters and isn't implied by Vec index
}

/// Represents how and where a logical piece of data (identified by user key) is stored.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyStorageInfo {
    /// List of pads holding the data for this key (Address, Key Bytes)
    pub pads: Vec<PadInfo>,
    /// Actual size of the data stored across these pads
    pub data_size: usize,
    /// Timestamp of the last modification (store or update)
    #[serde(default = "Utc::now")]
    pub modified: DateTime<Utc>,
    pub data_checksum: String,
    #[serde(default)] // Assume incomplete if field is missing during deserialization
    pub is_complete: bool,
    #[serde(default)] // Number of pads confirmed written
    pub populated_pads_count: usize,
}

/// The main index mapping user keys to their storage information.
pub type MasterIndex = HashMap<String, KeyStorageInfo>;

/// Stores the master index information, including free pads and the main index.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MasterIndexStorage {
    /// List of pads that are known to exist but are not currently assigned to any key.
    #[serde(default)]
    pub free_pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Address and Key
    /// List of pads from deleted keys that were not fully populated and need verification.
    #[serde(default)]
    pub pending_verification_pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Address and Key
    /// The main index mapping user keys to their storage details.
    #[serde(default)]
    pub index: HashMap<String, KeyStorageInfo>,
    /// The size of each scratchpad in bytes, determined on initialization.
    pub scratchpad_size: usize,
}
