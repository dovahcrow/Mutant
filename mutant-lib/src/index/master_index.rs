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
                    p.checksum = 0;
                    p.size = 0;
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
                IndexEntry::PrivateKey(pads) => {
                    if pads.len() != new_checksums.len() {
                        return false;
                    }
                    pads.iter()
                        .zip(new_checksums.iter())
                        .all(|(p, c)| p.checksum == *c)
                }
                IndexEntry::PublicUpload(index_pad, pads) => {
                    if pads.len() != new_checksums.len() {
                        return false;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ScratchpadAddress;

    #[test]
    fn test_new_master_index() {
        let index = MasterIndex::new();
        assert!(index.index.is_empty());
        assert!(index.free_pads.is_empty());
        assert!(index.pending_verification_pads.is_empty());
    }

    #[test]
    fn test_create_private_key_new() {
        let mut index = MasterIndex::new();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE * 2 + 10]; // Data spanning more than 2 pads
        let key_name = "test_key";

        let pads = index.create_private_key(key_name, &data).unwrap();

        assert_eq!(pads.len(), 3); // Should create 3 pads
        assert!(index.contains_key(key_name));
        if let Some(IndexEntry::PrivateKey(stored_pads)) = index.index.get(key_name) {
            assert_eq!(stored_pads.len(), 3);
            assert_eq!(stored_pads[0].status, PadStatus::Generated);
            assert_eq!(stored_pads[1].status, PadStatus::Generated);
            assert_eq!(stored_pads[2].status, PadStatus::Generated);
        } else {
            panic!("Key not found or not a PrivateKey entry");
        }
    }

    #[test]
    fn test_create_private_key_existing() {
        let mut index = MasterIndex::new();
        let data = vec![1u8; 10];
        let key_name = "test_key";

        index.create_private_key(key_name, &data).unwrap();
        let result = index.create_private_key(key_name, &data);

        assert!(matches!(
            result,
            Err(Error::Index(IndexError::KeyAlreadyExists(_)))
        ));
    }

    #[test]
    fn test_update_pad_status_private() {
        let mut index = MasterIndex::new();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        let key_name = "test_key";
        let pads = index.create_private_key(key_name, &data).unwrap();
        let pad_address = pads[0].address;

        index.update_pad_status(key_name, &pad_address, PadStatus::Confirmed);

        if let Some(IndexEntry::PrivateKey(updated_pads)) = index.index.get(key_name) {
            assert_eq!(updated_pads[0].status, PadStatus::Confirmed);
        } else {
            panic!("Key not found or not a PrivateKey entry");
        }
    }

    #[test]
    fn test_contains_key() {
        let mut index = MasterIndex::new();
        let data = vec![1u8; 10];
        let key_name = "test_key";

        assert!(!index.contains_key(key_name));
        index.create_private_key(key_name, &data).unwrap();
        assert!(index.contains_key(key_name));
        assert!(!index.contains_key("other_key"));
    }

    #[test]
    fn test_remove_key_moves_pads() {
        let mut index = MasterIndex::new();
        let data_gen = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        let data_conf = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let key_gen = "key_gen";
        let key_conf = "key_conf";

        let pads_gen = index.create_private_key(key_gen, &data_gen).unwrap();
        let pad_gen_addr = pads_gen[0].address;

        let pads_conf = index.create_private_key(key_conf, &data_conf).unwrap();
        let pad_conf_addr = pads_conf[0].address;
        index.update_pad_status(key_conf, &pad_conf_addr, PadStatus::Confirmed); // Mark as non-generated

        // Remove the key with the 'Generated' pad
        index.remove_key(key_gen).unwrap();
        assert!(!index.contains_key(key_gen));
        assert_eq!(index.pending_verification_pads.len(), 1);
        assert_eq!(index.pending_verification_pads[0].address, pad_gen_addr);
        assert_eq!(index.free_pads.len(), 0);

        // Remove the key with the 'Confirmed' pad
        index.remove_key(key_conf).unwrap();
        assert!(!index.contains_key(key_conf));
        assert_eq!(index.pending_verification_pads.len(), 1); // Should still have the one from key_gen
        assert_eq!(index.free_pads.len(), 1);
        assert_eq!(index.free_pads[0].address, pad_conf_addr);
        assert_eq!(index.free_pads[0].status, PadStatus::Free);
    }

    #[test]
    fn test_is_finished() {
        let mut index = MasterIndex::new();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE * 2];
        let key_name = "test_key";

        let pads = index.create_private_key(key_name, &data).unwrap();
        assert!(!index.is_finished(key_name)); // Not finished initially

        // Update one pad
        index.update_pad_status(key_name, &pads[0].address, PadStatus::Confirmed);
        assert!(!index.is_finished(key_name)); // Still not finished

        // Update the second pad
        index.update_pad_status(key_name, &pads[1].address, PadStatus::Confirmed);
        assert!(index.is_finished(key_name)); // Now finished

        assert!(!index.is_finished("non_existent_key")); // Test non-existent key
    }

    #[test]
    fn test_verify_checksum_private() {
        let mut index = MasterIndex::new();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE + 5];
        let key_name = "test_key";

        index.create_private_key(key_name, &data).unwrap();

        // Verify with correct data
        assert!(index.verify_checksum(key_name, &data));

        // Verify with incorrect data (different length)
        let wrong_data_len = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        assert!(!index.verify_checksum(key_name, &wrong_data_len));

        // Verify with incorrect data (same length, different content)
        let mut wrong_data_content = data.clone();
        wrong_data_content[0] = 1; // Change one byte
        assert!(!index.verify_checksum(key_name, &wrong_data_content));

        // Verify non-existent key
        assert!(!index.verify_checksum("non_existent_key", &data));
    }

    #[test]
    fn test_list() {
        let mut index = MasterIndex::new();
        assert!(index.list().is_empty());

        index.create_private_key("key1", &[1]).unwrap();
        index.create_private_key("key2", &[2]).unwrap();

        let mut keys = index.list();
        keys.sort(); // Sort for predictable order

        assert_eq!(keys, vec!["key1".to_string(), "key2".to_string()]);
    }

    #[test]
    fn test_aquire_pads_reuse_free() {
        let mut index = MasterIndex::new();
        let data1 = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let data2 = vec![2u8; DEFAULT_SCRATCHPAD_SIZE];
        let key1 = "key1";
        let key2 = "key2";

        // Create first key, its pad gets generated
        let pads1 = index.create_private_key(key1, &data1).unwrap();
        assert_eq!(pads1.len(), 1);
        let pad1_addr = pads1[0].address;
        index.update_pad_status(key1, &pad1_addr, PadStatus::Confirmed); // Mark as used

        // Remove first key, pad should go to free_pads
        index.remove_key(key1).unwrap();
        assert_eq!(index.free_pads.len(), 1);
        assert_eq!(index.pending_verification_pads.len(), 0);
        assert_eq!(index.free_pads[0].address, pad1_addr);
        assert_eq!(index.free_pads[0].status, PadStatus::Free); // Status updated

        // Create second key, should reuse the free pad
        let pads2 = index.create_private_key(key2, &data2).unwrap();
        assert_eq!(pads2.len(), 1);
        assert_eq!(pads2[0].address, pad1_addr); // Reused the same address
        assert_eq!(pads2[0].status, PadStatus::Free); // Status reset
        assert_eq!(pads2[0].checksum, PadInfo::checksum(&data2)); // Checksum updated
        assert!(index.free_pads.is_empty()); // Free pad list is now empty
    }

    #[test]
    fn test_aquire_pads_generate_new() {
        let mut index = MasterIndex::new();
        let data = vec![1u8; DEFAULT_SCRATCHPAD_SIZE * 2];
        let key = "key";

        // No free pads initially
        assert!(index.free_pads.is_empty());

        let pads = index.create_private_key(key, &data).unwrap();
        assert_eq!(pads.len(), 2);
        assert_ne!(pads[0].address, pads[1].address); // Ensure different addresses for generated pads
        assert!(index.free_pads.is_empty()); // Still no free pads
    }

    #[test]
    fn test_aquire_pads_mix_free_and_new() {
        let mut index = MasterIndex::new();
        let data1 = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let data3 = vec![3u8; DEFAULT_SCRATCHPAD_SIZE * 3]; // Requires 3 pads
        let key1 = "key1";
        let key3 = "key3";

        // Create key1, mark pad as used, remove key -> pad goes to free_pads
        let pads1 = index.create_private_key(key1, &data1).unwrap();
        let pad1_addr = pads1[0].address;
        index.update_pad_status(key1, &pad1_addr, PadStatus::Confirmed);
        index.remove_key(key1).unwrap();
        assert_eq!(index.free_pads.len(), 1);

        // Create key3, requires 3 pads. Should use 1 free pad and generate 2 new ones.
        let pads3 = index.create_private_key(key3, &data3).unwrap();
        assert_eq!(pads3.len(), 3);
        assert!(index.free_pads.is_empty()); // Free pad was used

        // Check if the first pad reused the free one
        assert_eq!(pads3[0].address, pad1_addr);
        assert_eq!(pads3[0].status, PadStatus::Free);
        assert_eq!(
            pads3[0].checksum,
            PadInfo::checksum(&data3[0..DEFAULT_SCRATCHPAD_SIZE])
        );

        // Check the newly generated pads
        assert_ne!(pads3[1].address, pad1_addr);
        assert_ne!(pads3[2].address, pad1_addr);
        assert_eq!(pads3[1].status, PadStatus::Generated);
        assert_eq!(pads3[2].status, PadStatus::Generated);
        assert_eq!(
            pads3[1].checksum,
            PadInfo::checksum(&data3[DEFAULT_SCRATCHPAD_SIZE..DEFAULT_SCRATCHPAD_SIZE * 2])
        );
        assert_eq!(
            pads3[2].checksum,
            PadInfo::checksum(&data3[DEFAULT_SCRATCHPAD_SIZE * 2..])
        );

        if let Some(IndexEntry::PrivateKey(stored_pads)) = index.index.get(key3) {
            assert_eq!(stored_pads.len(), 3);
        } else {
            panic!("Key not found or not a PrivateKey entry");
        }
    }
}
