use super::PadManager;
use crate::error::Error;
use crate::mutant::data_structures::PadUploadStatus;
use log::debug;

impl PadManager {
    /// Removes a key from the index and adds its associated pads to the appropriate list:
    /// `free_pads` if populated, `pending_verification_pads` otherwise.
    pub async fn release_pads(&self, key: &str) -> Result<(), Error> {
        debug!("PadManager::Release[{}]: Releasing pads...", key);
        let mut mis_guard = self.master_index_storage.lock().await;
        debug!("Release[{}]: Lock acquired.", key);

        match mis_guard.index.remove(key) {
            Some(key_info) => {
                let mut num_freed = 0;
                let mut num_pending = 0;
                for pad_info in key_info.pads {
                    let pad_tuple = (pad_info.address, pad_info.key);
                    if pad_info.status == PadUploadStatus::Populated
                        || pad_info.status == PadUploadStatus::Free
                    {
                        mis_guard.free_pads.push(pad_tuple);
                        num_freed += 1;
                    } else {
                        mis_guard.pending_verification_pads.push(pad_tuple);
                        num_pending += 1;
                    }
                }
                debug!(
                    "Release[{}]: Key removed. Added {} pads to free list (new total: {}), {} pads to pending verification (new total: {}). Releasing lock.",
                    key,
                    num_freed,
                    mis_guard.free_pads.len(),
                    num_pending,
                    mis_guard.pending_verification_pads.len()
                );
                Ok(())
            }
            None => {
                debug!("Release[{}]: Key not found. Releasing lock.", key);
                Err(Error::KeyNotFound(key.to_string()))
            }
        }
    }
}
