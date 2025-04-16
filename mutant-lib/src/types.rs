use crate::network::NetworkChoice; // Assuming NetworkChoice might be part of config later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// --- Public Configuration ---

// Re-export NetworkChoice for convenience if needed at top level

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutAntConfig {
    pub network: NetworkChoice,
    // Add other config options here, e.g., cache path, default chunk size?
}

impl Default for MutAntConfig {
    fn default() -> Self {
        Self {
            network: NetworkChoice::Mainnet, // Default to Mainnet as per original
        }
    }
}

// --- Public Data Structures ---

/// Holds detailed statistics about the storage usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    /// The configured usable size of each individual scratchpad in bytes.
    pub scratchpad_size: usize,
    /// The total number of scratchpads managed (occupied + free + pending).
    pub total_pads: usize,
    /// The number of scratchpads currently holding data.
    pub occupied_pads: usize,
    /// The number of scratchpads available for reuse.
    pub free_pads: usize,
    /// The number of scratchpads pending network verification.
    pub pending_verification_pads: usize,
    /// The total storage capacity across all managed scratchpads (total_pads * scratchpad_size).
    pub total_space_bytes: u64,
    /// The storage capacity currently used by occupied scratchpads (occupied_pads * scratchpad_size).
    pub occupied_pad_space_bytes: u64,
    /// The storage capacity currently available in free scratchpads (free_pads * scratchpad_size).
    pub free_pad_space_bytes: u64,
    /// The actual total size of the data stored across all occupied scratchpads.
    pub occupied_data_bytes: u64,
    /// The difference between the space allocated by occupied pads and the actual data stored (internal fragmentation).
    pub wasted_space_bytes: u64,

    // --- Stats specific to incomplete uploads ---
    /// The number of keys that are currently incomplete.
    pub incomplete_keys_count: usize,
    /// The total data size (in bytes) represented by incomplete keys.
    pub incomplete_keys_data_bytes: u64,
    /// The total number of pads associated with incomplete keys.
    pub incomplete_keys_total_pads: usize,
    /// The number of pads for incomplete keys still in the 'Generated' state (write pending).
    pub incomplete_keys_pads_generated: usize,
    /// The number of pads for incomplete keys in the 'Written' state (confirmation pending).
    pub incomplete_keys_pads_written: usize,
    /// The number of pads for incomplete keys already in the 'Confirmed' state.
    pub incomplete_keys_pads_confirmed: usize,
}

/// Holds detailed information about a specific user key stored in MutAnt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyDetails {
    pub key: String,
    pub size: usize,
    pub modified: DateTime<Utc>,
    pub is_finished: bool,
    pub completion_percentage: Option<f32>, // Percentage (0.0-100.0) if not finished
}
