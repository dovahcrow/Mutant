use crate::data::error::DataError;

use log::{debug, error, info, warn};

/// Removes data associated with a user key from the index.
///
/// This operation removes the key's metadata from the index and attempts
/// to mark the associated pads as free (harvesting) for potential reuse.
/// It does *not* actively delete data from the network storage itself.
///
/// # Arguments
///
/// * `data_manager` - A reference to the `DefaultDataManager` instance.
/// * `user_key` - The key identifying the data to remove.
///
/// # Errors
///
/// Returns `DataError` if:
/// - An index manager error occurs during key removal or pad harvesting (`DataError::Index`).
pub(crate) async fn remove_op(
    data_manager: &crate::data::manager::DefaultDataManager,
    user_key: &str,
) -> Result<(), DataError> {
    info!("DataOps: Starting remove operation for key '{}'", user_key);

    let removed_info = data_manager.index_manager.remove_key_info(user_key).await?;

    match removed_info {
        Some(key_info) => {
            debug!("Removed key info for '{}'. Harvesting pads...", user_key);
            if let Err(e) = data_manager.index_manager.harvest_pads(key_info).await {
                error!("Failed to harvest pads for key '{}': {}", user_key, e);
            }
            info!("DataOps: Remove operation complete for key '{}'.", user_key);
            Ok(())
        }
        None => {
            warn!("Attempted to remove non-existent key '{}'", user_key);
            Err(DataError::KeyNotFound(user_key.to_string()))
        }
    }
}
