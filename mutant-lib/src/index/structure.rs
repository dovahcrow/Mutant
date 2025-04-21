use crate::pad_lifecycle::PadOrigin;
use autonomi::client::payment::Receipt;
use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default size for scratchpads in bytes (4 MiB minus one page for metadata).
pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (4 * 1024 * 1024) - 4096;

/// Status of an individual pad within a key.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PadStatus {
    /// The pad has been generated but not yet allocated on a device.
    Generated,

    /// The pad has been allocated on a device but not yet written to.
    Allocated,

    /// The pad has been written to the device.
    Written,

    /// The pad's content has been verified against the original data.
    Confirmed,
}

/// Information about a single pad associated with a key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    /// The address of the scratchpad where this pad resides.
    pub address: ScratchpadAddress,

    /// The index of the chunk this pad represents within the original data.
    pub chunk_index: usize,

    /// The size stored on the pad.
    pub size: usize,

    /// The current status of this pad.
    pub status: PadStatus,

    /// The last counter value read from the network for this pad during confirmation.
    #[serde(default)] // For compatibility with older index formats. Defaults to 0 if missing.
    pub last_known_counter: u64,

    /// The secret key bytes used to create/update the scratchpad.
    /// Added for unified pad management and potential reuse.
    #[serde(default)] // Default for compatibility with older index formats
    pub sk_bytes: Vec<u8>,

    /// The payment receipt obtained when the pad was first paid for.
    /// Stored when a pad is initially created via scratchpad_put or scratchpad_create.
    /// Used for subsequent updates via PaymentOption::Receipt.
    #[serde(default)] // Default for compatibility with older index formats
    pub receipt: Option<Receipt>,
}

/// Information about a specific key stored in the system.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// List of pads associated with this key.
    pub pads: Vec<PadInfo>,

    /// The total size of the data associated with this key.
    pub data_size: usize,

    /// Timestamp of the last modification to this key's information.
    pub modified: DateTime<Utc>,

    /// Flag indicating if all pads for this key have been confirmed.
    pub is_complete: bool,
}

/// Information about a publicly uploaded entry in the index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicUploadInfo {
    /// Details of the index scratchpad for this public upload.
    pub index_pad: PadInfo,
    /// The total size of the publicly uploaded data in bytes.
    pub size: usize,
    /// Timestamp indicating when the public upload was created or last modified.
    pub modified: DateTime<Utc>,
    /// Details of the data pads storing the actual chunks.
    pub data_pads: Vec<PadInfo>,
}

/// Represents an entry in the master index, which can be either private key data or public upload data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum IndexEntry {
    PrivateKey(KeyInfo),
    PublicUpload(PublicUploadInfo),
}

/// The central index managing all keys and scratchpads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MasterIndex {
    /// Mapping from key names (e.g., file paths or public upload IDs) to their detailed information.
    pub index: HashMap<String, IndexEntry>,

    /// List of scratchpads that are currently free and available for allocation.
    /// Each tuple contains the address, the associated encryption key, and the generation ID.
    pub free_pads: Vec<(ScratchpadAddress, Vec<u8>, u64)>,

    /// List of scratchpads that are awaiting verification.
    /// Each tuple contains the address and the associated encryption key.
    pub pending_verification_pads: Vec<(ScratchpadAddress, Vec<u8>)>,

    /// The size of each scratchpad in bytes.
    pub scratchpad_size: usize,
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
