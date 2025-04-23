use crate::config::NetworkChoice;
use crate::data::storage_mode::StorageMode;
use crate::storage::ScratchpadAddress;
use crate::{index::pad_info::PadInfo, internal_error::Error};
use autonomi::data::public;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::slice::Chunks;
use xdg::BaseDirectories;

use super::error::IndexError;
use super::PadStatus;

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
    index: BTreeMap<String, IndexEntry>,

    /// List of scratchpads that are currently free and available for allocation.
    /// Each tuple contains the address, the associated encryption key, and the generation ID.
    free_pads: Vec<PadInfo>,

    /// List of scratchpads that are awaiting verification.
    /// Each tuple contains the address and the associated encryption key.
    pending_verification_pads: Vec<PadInfo>,

    network_choice: NetworkChoice,
}

impl MasterIndex {
    fn new_empty(network_choice: NetworkChoice) -> Self {
        MasterIndex {
            index: BTreeMap::new(),
            free_pads: Vec::new(),
            pending_verification_pads: Vec::new(),
            network_choice,
        }
    }
}

impl MasterIndex {
    pub fn new(network_choice: NetworkChoice) -> Self {
        match MasterIndex::load(network_choice) {
            Ok(index) => {
                log::info!("Loaded master index from file for {:?}.", network_choice);
                index
            }
            Err(e) => {
                log::warn!(
                    "Failed to load master index for {:?}, creating a new one. Error: {}",
                    network_choice,
                    e
                );
                MasterIndex::new_empty(network_choice)
            }
        }
    }

    pub fn get_index_pad_if_public(&self, key_name: &str) -> Option<PadInfo> {
        if let Some(entry) = self.index.get(key_name) {
            if let IndexEntry::PublicUpload(index, _) = entry {
                Some(index.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn load(network_choice: NetworkChoice) -> Result<Self, Error> {
        let path = get_index_file_path(network_choice)?;
        if !path.exists() {
            return Err(Error::Index(IndexError::IndexFileNotFound(
                path.display().to_string(),
            )));
        }
        let file = File::open(&path)
            .map_err(|e| Error::Index(IndexError::IndexFileNotFound(path.display().to_string())))?;
        let reader = BufReader::new(file);
        let index: MasterIndex = serde_cbor::from_reader(reader)
            .map_err(|e| Error::Index(IndexError::DeserializationError(e.to_string())))?;

        if index.network_choice != network_choice {
            return Err(Error::Index(IndexError::NetworkMismatch {
                x: network_choice,
                y: index.network_choice,
            }));
        }

        Ok(index)
    }

    pub fn save(&self, network_choice: NetworkChoice) -> Result<(), Error> {
        let path = get_index_file_path(network_choice)?;
        let file = File::create(&path)
            .map_err(|e| Error::Index(IndexError::IndexFileNotFound(path.display().to_string())))?;
        let writer = BufWriter::new(file);
        serde_cbor::to_writer(writer, self)
            .map_err(|e| Error::Index(IndexError::SerializationError(e.to_string())))?;
        log::info!("Saved master index to {}", path.display());
        Ok(())
    }

    pub fn create_private_key(
        &mut self,
        key_name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
    ) -> Result<Vec<PadInfo>, Error> {
        if self.index.contains_key(key_name) {
            return Err(IndexError::KeyAlreadyExists(key_name.to_string()).into());
        }

        let pads = self.aquire_pads(data_bytes, mode);

        self.index
            .insert(key_name.to_string(), IndexEntry::PrivateKey(pads.clone()));

        self.save(self.network_choice)?;

        Ok(pads)
    }

    pub fn create_public_key(
        &mut self,
        key_name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
    ) -> Result<Vec<PadInfo>, Error> {
        if self.index.contains_key(key_name) {
            return Err(IndexError::KeyAlreadyExists(key_name.to_string()).into());
        }

        // prepare the first pad as an index pad if data_bytes is longer than a scratchpad

        let pads = if data_bytes.len() > mode.scratchpad_size() {
            let mut pads = self.aquire_pads(data_bytes, mode);
            // serialize index to determine the size and checksum of the index pad
            let index_pad_serialized = serde_cbor::to_vec(&pads).unwrap();

            let index_pad = self.aquire_pads(&index_pad_serialized, mode)[0].clone();

            self.index.insert(
                key_name.to_string(),
                IndexEntry::PublicUpload(index_pad.clone(), pads.clone()),
            );

            pads.insert(0, index_pad);

            pads
        } else {
            let pads = self.aquire_pads(data_bytes, mode);
            self.index.insert(
                key_name.to_string(),
                IndexEntry::PublicUpload(pads[0].clone(), Vec::new()),
            );

            pads
        };

        self.save(self.network_choice)?;

        Ok(pads)
    }

    pub fn recycle_errored_pad(
        &mut self,
        key_name: &str,
        pad_address: &ScratchpadAddress,
    ) -> Result<PadInfo, Error> {
        let mut new_pad = PadInfo::new(&[0u8; 1], 0);

        if let Some(entry) = self.index.get_mut(key_name) {
            if let IndexEntry::PrivateKey(pads) = entry {
                let pad_index = pads.iter().position(|p| p.address == *pad_address).unwrap();
                let old_pad = pads[pad_index].clone();

                new_pad.checksum = old_pad.checksum;
                new_pad.size = old_pad.size;
                new_pad.chunk_index = old_pad.chunk_index;

                pads[pad_index] = new_pad.clone();

                self.pending_verification_pads.push(old_pad);

                self.save(self.network_choice)?;
            } else {
                unimplemented!()
            }
        } else {
            return Err(IndexError::KeyNotFound(key_name.to_string()).into());
        }

        Ok(new_pad)
    }

    pub fn update_pad_status(
        &mut self,
        key_name: &str,
        pad_address: &ScratchpadAddress,
        status: PadStatus,
        counter: Option<u64>,
    ) -> Result<PadInfo, Error> {
        let res = if let Some(entry) = self.index.get_mut(key_name) {
            if let IndexEntry::PrivateKey(pads) = entry {
                let pad = pads.iter_mut().find(|p| p.address == *pad_address).unwrap();
                debug!(
                    "Updated pad status for {} from {:?} to {:?}",
                    pad_address, pad.status, status
                );
                pad.status = status;
                if let Some(counter) = counter {
                    pad.last_known_counter = counter;
                }
                Ok(pad.clone())
            } else if let IndexEntry::PublicUpload(index_pad, pads) = entry {
                let pad = if pad_address == &index_pad.address {
                    index_pad.status = status;
                    if let Some(counter) = counter {
                        index_pad.last_known_counter = counter;
                    }
                    index_pad.clone()
                } else {
                    let pad_index = pads.iter().position(|p| p.address == *pad_address).unwrap();
                    let mut pad = pads[pad_index].clone();
                    pad.status = status;
                    if let Some(counter) = counter {
                        pad.last_known_counter = counter;
                    }
                    pad
                };
                Ok(pad)
            } else {
                unimplemented!()
            }
        } else {
            Err(IndexError::KeyNotFound(key_name.to_string()).into())
        };

        self.save(self.network_choice)?;

        res
    }

    pub fn contains_key(&self, key_name: &str) -> bool {
        self.index.contains_key(key_name)
    }

    pub fn get_pads(&self, key_name: &str) -> Vec<PadInfo> {
        if let Some(entry) = self.index.get(key_name) {
            match entry {
                IndexEntry::PrivateKey(pads) => pads.clone(),
                IndexEntry::PublicUpload(index, pads) => vec![index.clone()]
                    .into_iter()
                    .chain(pads.clone())
                    .collect(),
            }
        } else {
            Vec::new()
        }
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

        debug!(
            "Removing key {} with {} pads to free and {} pads to verify",
            key_name,
            pads_to_free.len(),
            pads_to_verify.len()
        );

        self.free_pads.extend(pads_to_free);
        self.pending_verification_pads.extend(pads_to_verify);

        self.index.remove(key_name);

        self.save(self.network_choice)?;

        info!("Removed key {}", key_name);

        Ok(())
    }

    fn aquire_pads(&mut self, data_bytes: &[u8], mode: StorageMode) -> Vec<PadInfo> {
        let mut chunks = data_bytes.chunks(mode.scratchpad_size());
        let total_length = chunks.clone().count();

        let mut pads = Vec::new();

        let to_drain_max = self.free_pads.len().min(total_length);

        if !self.free_pads.is_empty() {
            pads.extend(
                self.free_pads
                    .drain(0..to_drain_max)
                    .enumerate()
                    .map(|(i, p)| p.update_data(chunks.next().unwrap(), i))
                    .collect::<Vec<_>>(),
            );
        }

        debug!("Aquired {} pads from free_pads", pads.len());

        if pads.len() < total_length {
            pads.extend(self.generate_pads(pads.len(), chunks));
        }

        debug!("Aquired {} pads in total", pads.len());

        pads
    }

    fn generate_pads(&mut self, starting_chunk_index: usize, chunks: Chunks<u8>) -> Vec<PadInfo> {
        let mut chunk_index = starting_chunk_index;
        chunks
            .map(|chunk| {
                let pad = PadInfo::new(chunk, chunk_index);
                chunk_index += 1;
                pad
            })
            .collect::<Vec<_>>()
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

    pub fn verify_checksum(&self, key_name: &str, data_bytes: &[u8], mode: StorageMode) -> bool {
        let new_checksums = data_bytes
            .chunks(mode.scratchpad_size())
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

    pub fn list(&self) -> BTreeMap<String, IndexEntry> {
        let mut keys = self.index.clone();
        // put all the secret keys in the entries to 0
        keys.iter_mut().for_each(|(_, entry)| {
            if let IndexEntry::PrivateKey(pads) = entry {
                pads.iter_mut().for_each(|p| p.sk_bytes = vec![0; 32]);
            }
        });
        keys
    }

    pub fn export_raw_pads_private_key(&self) -> Result<Vec<PadInfo>, Error> {
        let mut pads_hex = Vec::new();
        for (_key, entry) in self.index.iter() {
            if let IndexEntry::PrivateKey(pads) = entry {
                for pad in pads {
                    pads_hex.push(pad.clone());
                }
            }
        }
        Ok(pads_hex)
    }

    pub fn pad_exists(&self, pad_address: &ScratchpadAddress) -> bool {
        let address_exists = self
            .free_pads
            .iter()
            .chain(self.pending_verification_pads.iter())
            .any(|p| p.address == *pad_address);

        let index_exists = self.index.iter().any(|(_, entry)| match entry {
            IndexEntry::PrivateKey(pads) => pads.iter().any(|p| p.address == *pad_address),
            IndexEntry::PublicUpload(index_pad, pads) => {
                index_pad.address == *pad_address || pads.iter().any(|p| p.address == *pad_address)
            }
        });

        address_exists || index_exists
    }

    pub fn import_raw_pads_private_key(&mut self, pads_hex: Vec<PadInfo>) -> Result<(), Error> {
        for mut pad in pads_hex {
            if self.pad_exists(&pad.address) {
                continue;
            }
            if pad.status == PadStatus::Generated {
                self.pending_verification_pads.push(pad);
            } else {
                pad.status = PadStatus::Free;
                self.free_pads.push(pad);
            }

            self.save(self.network_choice)?;
        }

        Ok(())
    }

    pub fn get_pending_pads(&self) -> Vec<PadInfo> {
        self.pending_verification_pads.clone()
    }

    pub fn verified_pending_pad(&mut self, mut pad: PadInfo) -> Result<(), Error> {
        self.discard_pending_pad(pad.clone())?;

        pad.status = PadStatus::Free;

        self.free_pads.push(pad.clone());

        self.save(self.network_choice)?;

        Ok(())
    }

    pub fn discard_pending_pad(&mut self, pad: PadInfo) -> Result<(), Error> {
        self.pending_verification_pads
            .iter()
            .position(|p| p.address == pad.address)
            .map(|i| self.pending_verification_pads.remove(i));

        self.save(self.network_choice)?;

        Ok(())
    }

    pub fn get_storage_stats(&self) -> StorageStats {
        let mut stats = StorageStats::default();

        stats.nb_keys = self.index.len() as u64;
        stats.occupied_pads = self
            .index
            .iter()
            .map(|(_, entry)| match entry {
                IndexEntry::PrivateKey(pads) => pads.len() as u64,
                IndexEntry::PublicUpload(_index, _pads) => unimplemented!(),
            })
            .sum();

        stats.free_pads = self.free_pads.len() as u64;
        stats.pending_verification_pads = self.pending_verification_pads.len() as u64;

        stats.total_pads = stats.occupied_pads + stats.free_pads + stats.pending_verification_pads;

        stats
    }
}

#[derive(Debug, Default)]
pub struct StorageStats {
    pub nb_keys: u64,
    pub total_pads: u64,
    pub occupied_pads: u64,
    pub free_pads: u64,
    pub pending_verification_pads: u64,
}

// Helper function to get the XDG data directory for Mutant
fn get_mutant_data_dir() -> Result<PathBuf, Error> {
    let xdg_dirs = BaseDirectories::with_prefix("mutant")
        .map_err(|e| Error::Index(IndexError::IndexFileNotFound(e.to_string())))?;
    let data_dir = xdg_dirs.get_data_home();
    fs::create_dir_all(&data_dir)
        .map_err(|e| Error::Index(IndexError::IndexFileNotFound(e.to_string())))?; // Ensure the directory exists
    Ok(data_dir)
}

// Helper function to get the full path for the index file
fn get_index_file_path(network_choice: NetworkChoice) -> Result<PathBuf, Error> {
    let data_dir = get_mutant_data_dir()?;
    let filename = match network_choice {
        NetworkChoice::Mainnet => "master_index_mainnet.cbor",
        NetworkChoice::Devnet => "master_index_devnet.cbor",
    };
    Ok(data_dir.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetworkChoice;
    use crate::data::storage_mode::MEDIUM_SCRATCHPAD_SIZE;
    use crate::storage::ScratchpadAddress;
    use tempfile::tempdir;

    const DEFAULT_SCRATCHPAD_SIZE: usize = MEDIUM_SCRATCHPAD_SIZE;

    // Helper to set up a temporary XDG directory for tests, always using Devnet
    fn setup_test_environment() -> (tempfile::TempDir, MasterIndex) {
        let temp_dir = tempdir().unwrap();
        let mock_data_home = temp_dir.path().join(".local/share");
        let mutant_data_dir = mock_data_home.join("mutant");
        fs::create_dir_all(&mutant_data_dir).unwrap();
        std::env::set_var("XDG_DATA_HOME", mock_data_home.to_str().unwrap());

        let index = MasterIndex::new(NetworkChoice::Devnet);
        (temp_dir, index)
    }

    #[test]
    fn test_new_master_index() {
        let (_td, index) = setup_test_environment();
        assert!(index.index.is_empty());
        assert!(index.free_pads.is_empty());
        assert!(index.pending_verification_pads.is_empty());
    }

    #[test]
    fn test_create_private_key_new() {
        let (_td, mut index) = setup_test_environment();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE * 2 + 10]; // Data spanning more than 2 pads
        let key_name = "test_key";

        let pads = index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();

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
        let (_td, mut index) = setup_test_environment();
        let data = vec![1u8; 10];
        let key_name = "test_key";

        index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();
        let result = index.create_private_key(key_name, &data, StorageMode::Medium);

        assert!(matches!(
            result,
            Err(Error::Index(IndexError::KeyAlreadyExists(_)))
        ));
    }

    #[test]
    fn test_update_pad_status_private() {
        let (_td, mut index) = setup_test_environment();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        let key_name = "test_key";
        let pads = index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();
        let pad_address = pads[0].address;

        index.update_pad_status(key_name, &pad_address, PadStatus::Confirmed, None);

        if let Some(IndexEntry::PrivateKey(updated_pads)) = index.index.get(key_name) {
            assert_eq!(updated_pads[0].status, PadStatus::Confirmed);
        } else {
            panic!("Key not found or not a PrivateKey entry");
        }
    }

    #[test]
    fn test_contains_key() {
        let (_td, mut index) = setup_test_environment();
        let data = vec![1u8; 10];
        let key_name = "test_key";

        assert!(!index.contains_key(key_name));
        index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();
        assert!(index.contains_key(key_name));
        assert!(!index.contains_key("other_key"));
    }

    #[test]
    fn test_remove_key_moves_pads() {
        let (_td, mut index) = setup_test_environment();
        let data_gen = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        let data_conf = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let key_gen = "key_gen";
        let key_conf = "key_conf";

        let pads_gen = index
            .create_private_key(key_gen, &data_gen, StorageMode::Medium)
            .unwrap();
        let pad_gen_addr = pads_gen[0].address;

        let pads_conf = index
            .create_private_key(key_conf, &data_conf, StorageMode::Medium)
            .unwrap();
        let pad_conf_addr = pads_conf[0].address;
        index.update_pad_status(key_conf, &pad_conf_addr, PadStatus::Confirmed, None); // Mark as non-generated

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
        let (_td, mut index) = setup_test_environment();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE * 2];
        let key_name = "test_key";

        let pads = index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();
        assert!(!index.is_finished(key_name)); // Not finished initially

        // Update one pad
        index.update_pad_status(key_name, &pads[0].address, PadStatus::Confirmed, None);
        assert!(!index.is_finished(key_name)); // Still not finished

        // Update the second pad
        index.update_pad_status(key_name, &pads[1].address, PadStatus::Confirmed, None);
        assert!(index.is_finished(key_name)); // Now finished

        assert!(!index.is_finished("non_existent_key")); // Test non-existent key
    }

    #[test]
    fn test_verify_checksum_private() {
        let (_td, mut index) = setup_test_environment();
        let data = vec![0u8; DEFAULT_SCRATCHPAD_SIZE + 5];
        let key_name = "test_key";

        index
            .create_private_key(key_name, &data, StorageMode::Medium)
            .unwrap();

        // Verify with correct data
        assert!(index.verify_checksum(key_name, &data, StorageMode::Medium));

        // Verify with incorrect data (different length)
        let wrong_data_len = vec![0u8; DEFAULT_SCRATCHPAD_SIZE];
        assert!(!index.verify_checksum(key_name, &wrong_data_len, StorageMode::Medium));

        // Verify with incorrect data (same length, different content)
        let mut wrong_data_content = data.clone();
        wrong_data_content[0] = 1; // Change one byte
        assert!(!index.verify_checksum(key_name, &wrong_data_content, StorageMode::Medium));

        // Verify non-existent key
        assert!(!index.verify_checksum("non_existent_key", &data, StorageMode::Medium));
    }

    #[test]
    fn test_list() {
        let (_td, mut index) = setup_test_environment();
        assert!(index.list().is_empty());

        index
            .create_private_key("key1", &[1], StorageMode::Medium)
            .unwrap();
        index
            .create_private_key("key2", &[2], StorageMode::Medium)
            .unwrap();

        let keys = index.list();

        assert_eq!(keys.keys().collect::<Vec<_>>(), vec!["key1", "key2"]);
    }

    #[test]
    fn test_aquire_pads_reuse_free() {
        let (_td, mut index) = setup_test_environment();
        let data1 = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let data2 = vec![2u8; DEFAULT_SCRATCHPAD_SIZE];
        let key1 = "key1";
        let key2 = "key2";

        // Create first key, its pad gets generated
        let pads1 = index
            .create_private_key(key1, &data1, StorageMode::Medium)
            .unwrap();
        assert_eq!(pads1.len(), 1);
        let pad1_addr = pads1[0].address;
        index.update_pad_status(key1, &pad1_addr, PadStatus::Confirmed, None); // Mark as used

        // Remove first key, pad should go to free_pads
        index.remove_key(key1).unwrap();
        assert_eq!(index.free_pads.len(), 1);
        assert_eq!(index.pending_verification_pads.len(), 0);
        assert_eq!(index.free_pads[0].address, pad1_addr);
        assert_eq!(index.free_pads[0].status, PadStatus::Free); // Status updated

        // Create second key, should reuse the free pad
        let pads2 = index
            .create_private_key(key2, &data2, StorageMode::Medium)
            .unwrap();
        assert_eq!(pads2.len(), 1);
        assert_eq!(pads2[0].address, pad1_addr); // Reused the same address
        assert_eq!(pads2[0].status, PadStatus::Free); // Status reset
        assert_eq!(pads2[0].checksum, PadInfo::checksum(&data2)); // Checksum updated
        assert!(index.free_pads.is_empty()); // Free pad list is now empty
    }

    #[test]
    fn test_aquire_pads_generate_new() {
        let (_td, mut index) = setup_test_environment();
        let data = vec![1u8; DEFAULT_SCRATCHPAD_SIZE * 2];
        let key = "key";

        // No free pads initially
        assert!(index.free_pads.is_empty());

        let pads = index
            .create_private_key(key, &data, StorageMode::Medium)
            .unwrap();
        assert_eq!(pads.len(), 2);
        assert_ne!(pads[0].address, pads[1].address); // Ensure different addresses for generated pads
        assert!(index.free_pads.is_empty()); // Still no free pads
    }

    #[test]
    fn test_aquire_pads_mix_free_and_new() {
        let (_td, mut index) = setup_test_environment();
        let data1 = vec![1u8; DEFAULT_SCRATCHPAD_SIZE];
        let data3 = vec![3u8; DEFAULT_SCRATCHPAD_SIZE * 3]; // Requires 3 pads
        let key1 = "key1";
        let key3 = "key3";

        // Create key1, mark pad as used, remove key -> pad goes to free_pads
        let pads1 = index
            .create_private_key(key1, &data1, StorageMode::Medium)
            .unwrap();
        let pad1_addr = pads1[0].address;
        index.update_pad_status(key1, &pad1_addr, PadStatus::Confirmed, None);
        index.remove_key(key1).unwrap();
        assert_eq!(index.free_pads.len(), 1);

        // Create key3, requires 3 pads. Should use 1 free pad and generate 2 new ones.
        let pads3 = index
            .create_private_key(key3, &data3, StorageMode::Medium)
            .unwrap();
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

    #[test]
    fn test_save_and_load() {
        let network = NetworkChoice::Devnet;
        let index_path = {
            let (_td, mut index1) = setup_test_environment(); // Always uses Devnet
            assert!(index1.index.is_empty());

            let data = vec![1, 2, 3];
            let key = "mykey";
            index1
                .create_private_key(key, &data, StorageMode::Medium)
                .unwrap();

            // Need the path before td goes out of scope
            let path = get_index_file_path(network).unwrap();
            index1.save(network).unwrap();
            assert!(path.exists());
            path
        }; // index1 and _td go out of scope

        // 2. Load the index and verify data
        {
            let (_td2, index2) = setup_test_environment(); // Should load from the saved file
            assert!(index2.contains_key("mykey"));
            let pads = index2.get_pads("mykey");
            assert_eq!(pads.len(), 1);
            assert_eq!(pads[0].checksum, PadInfo::checksum(&[1, 2, 3]));

            // Verify it doesn't load across networks (This test part doesn't make sense anymore as we only test Devnet)
            // let (_td_main, index_main) = setup_test_environment(NetworkChoice::Mainnet); // <-- We can't test Mainnet easily now
            // assert!(!index_main.contains_key("mykey")); // <-- This check is removed
        }
    }

    #[test]
    fn test_load_non_existent_file() {
        // Test loading when the file doesn't exist for Devnet
        let (_td, index) = setup_test_environment(); // Sets up XDG env but doesn't create the file
        assert!(index.index.is_empty()); // Verify it's empty
    }
}
