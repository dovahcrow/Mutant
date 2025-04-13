use super::MutAnt;
use crate::cache::write_local_index;
use crate::error::Error;
use log::{debug, error, info, warn};

/// Removes an item by user key, recycling its pads.
pub(super) async fn delete_item(es: &MutAnt, key: &str) -> Result<(), Error> {
    debug!("DeleteItem[{}]: Starting removal...", key);

    match es.pad_manager.release_pads(key).await {
        Ok(_) => {
            debug!(
                "DeleteItem[{}]: PadManager successfully released pads.",
                key
            );
        }
        Err(Error::KeyNotFound(_)) => {
            warn!("DeleteItem[{}]: Key not found by PadManager.", key);
            return Err(Error::KeyNotFound(key.to_string()));
        }
        Err(e) => {
            error!(
                "DeleteItem[{}]: Error releasing pads via PadManager: {}",
                key, e
            );
            return Err(e);
        }
    }

    debug!(
        "DeleteItem[{}]: Saving updated index to local cache...",
        key
    );
    let mis_guard = es.master_index_storage.lock().await;
    let index_to_cache = mis_guard.clone();
    drop(mis_guard);

    let network = es.get_network_choice();
    match write_local_index(&index_to_cache, network).await {
        Ok(_) => {
            debug!(
                "DeleteItem[{}]: Successfully wrote index to local cache after removal.",
                key
            );
        }
        Err(e) => {
            error!(
                "DeleteItem[{}]: Failed to write index to local cache after removal: {}",
                key, e
            );
            warn!(
                "Proceeding after failing to write local cache for key '{}' removal.",
                key
            );
        }
    }

    info!(
        "DeleteItem[{}]: Removal successful (in memory and cache attempted).",
        key
    );
    Ok(())
}
