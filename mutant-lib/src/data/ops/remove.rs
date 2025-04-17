// Remove operation logic
use crate::data::error::DataError;
// Remove unused import
// use crate::index::IndexManager;
use log::{debug, info, warn};

use super::common::DataManagerDependencies;

pub(crate) async fn remove_op(
    deps: &DataManagerDependencies,
    user_key: &str,
) -> Result<(), DataError> {
    info!("DataOps: Starting remove operation for key '{}'", user_key);

    // 1. Remove key info from index, getting the old info
    let removed_info = deps.index_manager.remove_key_info(user_key).await?;

    match removed_info {
        Some(_key_info) => {
            // Use _key_info as it's not needed after removal
            debug!("Removed key info for '{}' from index.", user_key);
            // The call to index_manager.remove_key_info above handles
            // sorting the pads associated with the removed key into
            // either the free_pads or pending_verification_pads list
            // based on their status. No explicit release call needed here.

            info!("DataOps: Remove operation complete for key '{}'.", user_key);
            Ok(())
        }
        None => {
            warn!("Attempted to remove non-existent key '{}'", user_key);
            // Return Ok or KeyNotFound? Original returned Ok. Let's stick with that.
            Ok(())
            // Err(DataError::KeyNotFound(user_key.to_string()))
        }
    }
}
