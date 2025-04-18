use crate::data::error::DataError;

use log::{debug, error, info, warn};

use super::common::DataManagerDependencies;

pub(crate) async fn remove_op(
    deps: &DataManagerDependencies,
    user_key: &str,
) -> Result<(), DataError> {
    info!("DataOps: Starting remove operation for key '{}'", user_key);

    let removed_info = deps.index_manager.remove_key_info(user_key).await?;

    match removed_info {
        Some(key_info) => {
            debug!("Removed key info for '{}'. Harvesting pads...", user_key);
            if let Err(e) = deps.index_manager.harvest_pads(key_info).await {
                error!("Failed to harvest pads for key '{}': {}", user_key, e);
            }
            info!("DataOps: Remove operation complete for key '{}'.", user_key);
        }
        None => {
            warn!("Attempted to remove non-existent key '{}'", user_key);
        }
    }
    Ok(())
}
