use super::PadManager;
use crate::error::Error;
use log::debug;

impl PadManager {
    /// Removes a key from the index and adds its associated pads to the free list.
    pub async fn release_pads(&self, key: &str) -> Result<(), Error> {
        debug!("PadManager::Release[{}]: Releasing pads...", key);
        let mut mis_guard = self.master_index_storage.lock().await;
        debug!("Release[{}]: Lock acquired.", key);

        match mis_guard.index.remove(key) {
            Some(key_info) => {
                let num_released = key_info.pads.len();
                mis_guard.free_pads.extend(key_info.pads);
                debug!(
                    "Release[{}]: Key removed. Added {} pads to free list (new total: {}). Releasing lock.",
                    key,
                    num_released,
                    mis_guard.free_pads.len()
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
