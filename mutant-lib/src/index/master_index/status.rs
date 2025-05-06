use crate::error::Error;
use crate::index::error::IndexError;
use crate::index::pad_info::PadInfo;
use crate::index::PadStatus;
use crate::storage::ScratchpadAddress;
use log::debug;
use mutant_protocol::StorageMode;

use super::{IndexEntry, MasterIndex};

impl MasterIndex {
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
                    let pad = pads.iter_mut().find(|p| p.address == *pad_address).unwrap();
                    pad.status = status;
                    if let Some(counter) = counter {
                        pad.last_known_counter = counter;
                    }
                    pad.clone()
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
                IndexEntry::PublicUpload(_index_pad, pads) => {
                    if pads.len() != new_checksums.len() {
                        return false;
                    }

                    pads.iter()
                        .zip(new_checksums.iter())
                        .all(|(p, c)| p.checksum == *c)
                }
            }
        } else {
            false
        }
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

    pub fn get_storage_stats(&self) -> super::StorageStats {
        let mut stats = super::StorageStats::default();

        stats.nb_keys = self.index.len() as u64;
        stats.occupied_pads = self
            .index
            .iter()
            .map(|(_, entry)| match entry {
                IndexEntry::PrivateKey(pads) => pads.len() as u64,
                IndexEntry::PublicUpload(_index, pads) => pads.len() as u64 + 1,
            })
            .sum();

        stats.free_pads = self.free_pads.len() as u64;
        stats.pending_verification_pads = self.pending_verification_pads.len() as u64;

        stats.total_pads = stats.occupied_pads + stats.free_pads + stats.pending_verification_pads;

        stats
    }
}
