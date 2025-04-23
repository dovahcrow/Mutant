use crate::network::NetworkError;
use crate::storage::ScratchpadAddress;
use blsttc::SecretKey;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Status of an individual pad within a key.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Copy)]
pub enum PadStatus {
    /// The pad has been generated but not yet allocated on a device.
    Generated,

    /// The pad has been allocated on a device but not yet written to.
    Free,

    /// The pad has been written to the device.
    Written,

    /// The pad's content has been verified against the original data.
    Confirmed,

    /// The pad has encountered repeated error during upload or download and must be recycled..
    Errored,
}

/// Information about a single pad associated with a key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    /// The address of the scratchpad where this pad resides.
    pub address: ScratchpadAddress,

    /// The size stored on the pad.
    pub size: usize,

    /// The chunk index of the pad.
    pub chunk_index: usize,

    /// The current status of this pad.
    pub status: PadStatus,

    /// The last counter value read from the network for this pad during confirmation.
    #[serde(default)] // For compatibility with older index formats. Defaults to 0 if missing.
    pub last_known_counter: u64,

    /// The secret key bytes used to create/update the scratchpad.
    pub sk_bytes: Vec<u8>,

    /// The checksum of unencrypted data stored on the pad.
    pub checksum: usize,
}

impl PadInfo {
    pub fn secret_key(&self) -> SecretKey {
        let mut secret_key_bytes: [u8; 32] = [0; 32];
        secret_key_bytes.copy_from_slice(&self.sk_bytes);
        SecretKey::from_bytes(secret_key_bytes)
            .map_err(|e| NetworkError::InternalError(format!("Failed to reconstruct SK: {}", e)))
            .unwrap()
    }
}

impl PadInfo {
    pub fn new(data: &[u8], chunk_index: usize) -> Self {
        let secret_key = SecretKey::random();
        let sk_bytes = secret_key.to_bytes().to_vec();
        let address = ScratchpadAddress::new(secret_key.public_key());
        Self {
            address,
            sk_bytes,
            size: data.len(),
            status: PadStatus::Generated,
            last_known_counter: 0,
            chunk_index,
            checksum: Self::checksum(data),
        }
    }

    pub fn update_data(mut self, data: &[u8]) -> Self {
        self.size = data.len();
        self.checksum = Self::checksum(data);
        self.last_known_counter += 1;
        self
    }

    pub fn update_status(&mut self, status: PadStatus) {
        self.status = status;
    }

    pub fn checksum(data: &[u8]) -> usize {
        let mut hasher = DefaultHasher::new();
        data.len().hash(&mut hasher);
        data.hash(&mut hasher);
        hasher.finish() as usize
    }
}
