use crate::network::NetworkChoice;
use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration settings for a MutAnt instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutAntConfig {
    /// Specifies the network (e.g., Mainnet, Testnet) to connect to.
    pub network: NetworkChoice,
}

impl Default for MutAntConfig {
    fn default() -> Self {
        Self {
            network: NetworkChoice::Mainnet,
        }
    }
}

/// Provides detailed statistics about the storage backend's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    /// Size of the scratchpad area in bytes, if applicable to the backend.
    pub scratchpad_size: usize,

    /// Total number of storage pads available in the backend.
    pub total_pads: usize,

    /// Number of pads currently occupied with data.
    pub occupied_pads: usize,

    /// Number of pads currently free and available for allocation.
    pub free_pads: usize,

    /// Number of pads that are pending verification or confirmation.
    pub pending_verification_pads: usize,

    /// Total storage capacity of the backend in bytes.
    pub total_space_bytes: u64,

    /// Total space in bytes consumed by occupied pads, including any overhead.
    pub occupied_pad_space_bytes: u64,

    /// Total space in bytes available within free pads.
    pub free_pad_space_bytes: u64,

    /// Total size in bytes of the actual user data stored within occupied pads.
    pub occupied_data_bytes: u64,

    /// Estimated space in bytes considered wasted due to fragmentation, padding, or other overhead.
    pub wasted_space_bytes: u64,

    /// Number of keys whose data is not yet fully stored or replicated across the network.
    pub incomplete_keys_count: usize,

    /// Total data size in bytes associated with incomplete keys.
    pub incomplete_keys_data_bytes: u64,

    /// Total number of pads allocated across all incomplete keys.
    pub incomplete_keys_total_pads: usize,

    /// Number of pads successfully generated for incomplete keys.
    pub incomplete_keys_pads_generated: usize,

    /// Number of pads successfully written to storage for incomplete keys.
    pub incomplete_keys_pads_written: usize,

    /// Number of pads confirmed (e.g., replicated or verified) for incomplete keys.
    pub incomplete_keys_pads_confirmed: usize,

    /// Number of pads allocated/written but not yet confirmed for incomplete keys.
    pub incomplete_keys_pads_allocated_written: usize,

    /// Number of public index entries (names).
    pub public_index_count: usize,

    /// Total space consumed by public index scratchpads (count * scratchpad_size).
    pub public_index_space_bytes: u64,

    /// Total number of unique data pads referenced across all public uploads.
    pub public_data_pad_count: usize,

    /// Total space consumed by unique public data pads (count * scratchpad_size).
    pub public_data_space_bytes: u64,

    /// Total size in bytes of the actual user data stored within public data pads.
    pub public_data_actual_bytes: u64,

    /// Estimated wasted space within public data pads (space_bytes - actual_bytes).
    pub public_data_wasted_bytes: u64,
}

/// Contains metadata about a specific key stored within MutAnt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyDetails {
    /// The unique identifier (key) for the stored data.
    pub key: String,
    /// The size of the data associated with the key, in bytes.
    pub size: usize,
    /// Timestamp indicating when the key's data was last modified.
    pub modified: DateTime<Utc>,
    /// Flag indicating whether the data associated with the key is fully stored and retrievable.
    pub is_finished: bool,
    /// If `is_finished` is false, this optionally provides the completion progress as a percentage (0.0 to 100.0).
    pub completion_percentage: Option<f32>,
    /// If this represents a public upload, this holds its shareable address.
    pub public_address: Option<ScratchpadAddress>,
}

/// Contains summarized information about a key for listing purposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySummary {
    /// The unique identifier (key or name) for the entry.
    pub name: String,
    /// Flag indicating whether this entry is a public upload.
    pub is_public: bool,
    /// The shareable address, if this entry is a public upload.
    pub address: Option<ScratchpadAddress>,
}
