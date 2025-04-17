use crate::data::error::DataError;

use log::{debug, info, warn};

use super::common::DataManagerDependencies;

pub(crate) async fn remove_op(
    deps: &DataManagerDependencies,
    user_key: &str,
) -> Result<(), DataError> {
    info!("DataOps: Starting remove operation for key '{}'", user_key);

    let removed_info = deps.index_manager.remove_key_info(user_key).await?;

    match removed_info {
        Some(_key_info) => {
            debug!("Removed key info for '{}' from index.", user_key);

            info!("DataOps: Remove operation complete for key '{}'.", user_key);
            Ok(())
        }
        None => {
            warn!("Attempted to remove non-existent key '{}'", user_key);

            Ok(())
        }
    }
}
