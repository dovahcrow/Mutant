use crate::storage::ScratchpadAddress;
use crate::{index::pad_info::PadInfo, internal_error::Error};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::slice::Chunks;

use super::error::IndexError;
use super::{PadStatus, DEFAULT_SCRATCHPAD_SIZE};

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
    index: HashMap<String, IndexEntry>,

    /// List of scratchpads that are currently free and available for allocation.
    /// Each tuple contains the address, the associated encryption key, and the generation ID.
    free_pads: Vec<PadInfo>,

    /// List of scratchpads that are awaiting verification.
    /// Each tuple contains the address and the associated encryption key.
    pending_verification_pads: Vec<PadInfo>,
}

impl Default for MasterIndex {
    fn default() -> Self {
        MasterIndex {
            index: HashMap::new(),
            free_pads: Vec::new(),
            pending_verification_pads: Vec::new(),
        }
    }
}

impl MasterIndex {
    pub fn new() -> Self {
        MasterIndex::default()
    }

    pub fn create_private_key(
        &mut self,
        key_name: &str,
        data_bytes: &[u8],
    ) -> Result<Vec<PadInfo>, Error> {
        if self.index.contains_key(key_name) {
            return Err(IndexError::KeyAlreadyExists(key_name.to_string()).into());
        }

        let pads = self.aquire_pads(data_bytes);

        self.index
            .insert(key_name.to_string(), IndexEntry::PrivateKey(pads.clone()));

        Ok(pads)
    }

    pub fn update_pad_status(
        &mut self,
        key_name: &str,
        pad_address: &ScratchpadAddress,
        status: PadStatus,
    ) {
        if let Some(entry) = self.index.get_mut(key_name) {
            if let IndexEntry::PrivateKey(pads) = entry {
                pads.iter_mut()
                    .find(|p| p.address == *pad_address)
                    .unwrap()
                    .status = status;
            } else if let IndexEntry::PublicUpload(index_pad, pads) = entry {
                if index_pad.address == *pad_address {
                    index_pad.status = status;
                } else {
                    pads.iter_mut()
                        .find(|p| p.address == *pad_address)
                        .unwrap()
                        .status = status;
                }
            }
        }
    }

    pub fn contains_key(&self, key_name: &str) -> bool {
        self.index.contains_key(key_name)
    }

    pub fn remove_key(&mut self, key_name: &str) -> Result<(), Error> {
        // for each pad that has a status different than Generated, we update their status to Free
        let mut pads_to_free = Vec::new();
        let mut pads_to_verify = Vec::new();

        if let Some(entry) = self.index.get_mut(key_name) {
            if let IndexEntry::PrivateKey(pads) = entry {
                pads.iter_mut().for_each(|p| {
                    if p.status != PadStatus::Generated {
                        p.status = PadStatus::Free;
                        pads_to_free.push(p.clone());
                    } else {
                        pads_to_verify.push(p.clone());
                    }
                });
            } else if let IndexEntry::PublicUpload(_, _) = entry {
                return Err(IndexError::CannotRemovePublicUpload(key_name.to_string()).into());
            }
        }

        self.free_pads.extend(pads_to_free);
        self.pending_verification_pads.extend(pads_to_verify);

        self.index.remove(key_name);

        Ok(())
    }

    fn aquire_pads(&mut self, data_bytes: &[u8]) -> Vec<PadInfo> {
        let mut chunks = data_bytes.chunks(DEFAULT_SCRATCHPAD_SIZE);
        let total_length = chunks.clone().count();

        let mut pads = Vec::new();
        pads.extend(
            self.free_pads
                .drain(0..)
                .map(|p| p.update_data(chunks.next().unwrap()))
                .collect::<Vec<_>>(),
        );

        if pads.len() < total_length {
            pads.extend(self.generate_pads(chunks));
        }

        pads
    }

    fn generate_pads(&mut self, chunks: Chunks<u8>) -> Vec<PadInfo> {
        chunks.map(|chunk| PadInfo::new(chunk)).collect::<Vec<_>>()
    }

    pub fn is_finished(&self, key_name: &str) -> bool {
        if let Some(entry) = self.index.get(key_name) {
            match entry {
                IndexEntry::PrivateKey(pads) => {
                    pads.iter().all(|p| p.status == PadStatus::Confirmed)
                }
                IndexEntry::PublicUpload(index_pad, pads) => {
                    pads.iter().all(|p| p.status == PadStatus::Confirmed)
                        && index_pad.status == PadStatus::Confirmed
                }
            }
        } else {
            false
        }
    }

    pub fn verify_checksum(&self, key_name: &str, data_bytes: &[u8]) -> bool {
        let new_checksums = data_bytes
            .chunks(DEFAULT_SCRATCHPAD_SIZE)
            .map(|chunk| PadInfo::checksum(chunk))
            .collect::<Vec<_>>();

        if let Some(entry) = self.index.get(key_name) {
            match entry {
                IndexEntry::PrivateKey(pads) => pads
                    .iter()
                    .zip(new_checksums.iter())
                    .all(|(p, c)| p.checksum == *c),
                IndexEntry::PublicUpload(index_pad, pads) => {
                    pads.iter()
                        .zip(new_checksums.iter())
                        .all(|(p, c)| p.checksum == *c)
                        && index_pad.checksum == new_checksums[0]
                }
            }
        } else {
            false
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.index.keys().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct KeysInfo {
    pub keys: Vec<String>,
}
