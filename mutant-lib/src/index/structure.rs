use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- Constants ---
// TODO: Consider making this configurable or dynamically determined later.
/// Default usable size within a scratchpad, leaving margin for overhead.
pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (4 * 1024 * 1024) - 512; // 4MB minus 512 bytes margin

// --- Enums ---

/// Represents the state of a scratchpad associated with a key's data chunk.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PadStatus {
    /// Key generated, creation/write pending or in progress.
    Generated,
    /// Network write call returned success, confirmation might be pending.
    Written,
    /// Write is confirmed on the network (final state for occupied pad).
    Confirmed,
}

// --- Core Index Structures ---

/// Represents information about a single scratchpad used to store a chunk of data for a key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    /// The network address of the scratchpad.
    pub address: ScratchpadAddress,
    /// The index of the data chunk stored in this pad (0-based).
    pub chunk_index: usize,
    /// The current status of this pad within the key's lifecycle.
    pub status: PadStatus,
    // Potentially add: `checksum: Option<[u8; 32]>` if implementing checksums
}

/// Represents the metadata associated with a single user key stored in MutAnt.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// List of pads holding the data chunks for this key, ordered by chunk_index.
    pub pads: Vec<PadInfo>,
    /// Map storing the secret keys (as bytes) for the pads used by this key.
    pub pad_keys: HashMap<ScratchpadAddress, Vec<u8>>,
    /// The total original size of the data in bytes.
    pub data_size: usize,
    /// Timestamp of the last modification.
    pub modified: DateTime<Utc>,
    /// Flag indicating if the data storage process was fully completed and confirmed.
    pub is_complete: bool,
    // Potentially add: `encryption_metadata: Option<Vec<u8>>`
}

/// The main structure holding all metadata managed by MutAnt.
/// This includes the mapping of user keys to their data layout and the pools of available pads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MasterIndex {
    /// Maps user-defined keys to their corresponding metadata and pad lists.
    pub index: HashMap<String, KeyInfo>,
    /// List of scratchpads (address and private key bytes) available for storing new data.
    pub free_pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Store key bytes directly
    /// List of scratchpads (address and private key bytes) that need network verification.
    pub pending_verification_pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    /// The usable size of scratchpads assumed by this index. Data is chunked based on this.
    pub scratchpad_size: usize,
    // Potentially add: `version: u32` for future format changes
}

impl Default for MasterIndex {
    fn default() -> Self {
        MasterIndex {
            index: HashMap::new(),
            free_pads: Vec::new(),
            pending_verification_pads: Vec::new(),
            scratchpad_size: DEFAULT_SCRATCHPAD_SIZE,
        }
    }
}
