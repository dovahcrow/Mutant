use super::Anthill;
use crate::error::Error;
use log::{debug, error, info, warn};

/// Removes an item by user key, recycling its pads.
pub(super) async fn delete_item(es: &Anthill, key: &str) -> Result<(), Error> {
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

    debug!("DeleteItem[{}]: Saving updated master index...", key);
    match es.save_master_index().await {
        Ok(_) => {
            debug!(
                "DeleteItem[{}]: Successfully persisted master index removal.",
                key
            );
        }
        Err(e) => {
            error!(
                "DeleteItem[{}]: CRITICAL: Failed to persist master index removal: {}. State might be inconsistent.",
                key, e
            );
            return Err(e);
        }
    }

    info!("DeleteItem[{}]: Removal successful.", key);
    Ok(())
}
