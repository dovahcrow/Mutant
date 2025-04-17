use crate::network::NetworkChoice;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutAntConfig {
    pub network: NetworkChoice,
}

impl Default for MutAntConfig {
    fn default() -> Self {
        Self {
            network: NetworkChoice::Mainnet,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    pub scratchpad_size: usize,

    pub total_pads: usize,

    pub occupied_pads: usize,

    pub free_pads: usize,

    pub pending_verification_pads: usize,

    pub total_space_bytes: u64,

    pub occupied_pad_space_bytes: u64,

    pub free_pad_space_bytes: u64,

    pub occupied_data_bytes: u64,

    pub wasted_space_bytes: u64,

    pub incomplete_keys_count: usize,

    pub incomplete_keys_data_bytes: u64,

    pub incomplete_keys_total_pads: usize,

    pub incomplete_keys_pads_generated: usize,

    pub incomplete_keys_pads_written: usize,

    pub incomplete_keys_pads_confirmed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyDetails {
    pub key: String,
    pub size: usize,
    pub modified: DateTime<Utc>,
    pub is_finished: bool,
    pub completion_percentage: Option<f32>,
}
