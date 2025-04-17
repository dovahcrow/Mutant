use crate::pad_lifecycle::PadOrigin;
use autonomi::ScratchpadAddress;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (4 * 1024 * 1024) - 4096;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PadStatus {
    Generated,

    Allocated,

    Written,

    Confirmed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    pub address: ScratchpadAddress,

    pub chunk_index: usize,

    pub status: PadStatus,

    pub origin: PadOrigin,

    #[serde(default)]
    pub needs_reverification: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub pads: Vec<PadInfo>,

    pub pad_keys: HashMap<ScratchpadAddress, Vec<u8>>,

    pub data_size: usize,

    pub modified: DateTime<Utc>,

    pub is_complete: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MasterIndex {
    pub index: HashMap<String, KeyInfo>,

    pub free_pads: Vec<(ScratchpadAddress, Vec<u8>, u64)>,

    pub pending_verification_pads: Vec<(ScratchpadAddress, Vec<u8>)>,

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
