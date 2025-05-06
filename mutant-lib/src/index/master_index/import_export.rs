use crate::error::Error;
use crate::index::pad_info::PadInfo;
use crate::index::PadStatus;

use super::{IndexEntry, MasterIndex};

impl MasterIndex {
    pub fn export_raw_pads_private_key(&self) -> Result<Vec<PadInfo>, Error> {
        let mut pads_hex = Vec::new();
        for (_key, entry) in self.index.iter() {
            match entry {
                IndexEntry::PrivateKey(pads) => {
                    for pad in pads {
                        pads_hex.push(pad.clone());
                    }
                }
                IndexEntry::PublicUpload(index, pads) => {
                    pads_hex.push(index.clone());
                    for pad in pads {
                        pads_hex.push(pad.clone());
                    }
                }
            }
        }
        pads_hex.extend(self.free_pads.clone());
        pads_hex.extend(self.pending_verification_pads.clone());

        Ok(pads_hex)
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
}
