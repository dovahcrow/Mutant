use super::{store_logic, MutAnt};
use crate::error::Error;
use crate::events::PutCallback;
use log::{debug, info, trace};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Updates an existing item with new data by performing a delete followed by a store.
///
/// If the key does not exist, it returns a `KeyNotFound` error.
pub(super) async fn update_item(
    ma: &MutAnt, // Renamed from es
    key: &str,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), Error> {
    let data_size = data_bytes.len();
    let key_owned = key.to_string();
    trace!(
        "MutAnt::UpdateItem[{}]: Starting update for key '{}' ({} bytes) via delete+store",
        ma.master_index_addr,
        key, // Use borrowed key for logging
        data_size
    );

    // 1. Check if key exists (lock MIS briefly)
    {
        let mis_guard = ma.master_index_storage.lock().await;
        if !mis_guard.index.contains_key(&key_owned) {
            debug!("UpdateItem[{}]: Key does not exist.", key_owned);
            return Err(Error::KeyNotFound(key_owned));
        }
    } // Lock released

    // 2. Delete the existing item
    debug!("UpdateItem[{}]: Deleting existing item...", key_owned);
    // Assuming delete_item handles cache persistence
    super::remove_logic::delete_item(ma, &key_owned).await.map_err(|e| {
        // Log the error but proceed to store if KeyNotFound (shouldn't happen after check, but be robust)
        // Propagate other errors
        if matches!(e, Error::KeyNotFound(_)) {
             debug!("UpdateItem[{}]: delete_item returned KeyNotFound unexpectedly after existence check. Proceeding with store.", key_owned);
             Error::InternalError(format!("Inconsistent state during update for key {}", key_owned))
        } else {
            e
        }
    })?;
    info!("UpdateItem[{}]: Existing item deleted.", key_owned);

    // 3. Store the new item data
    debug!("UpdateItem[{}]: Storing new item data...", key_owned);
    // Create a dummy commit counter Arc for the store call
    let commit_counter_arc = Arc::new(Mutex::new(0u64));
    // store_data handles planning, persistence, callbacks, etc.
    store_logic::store_data(ma, &key_owned, data_bytes, callback, commit_counter_arc).await?;

    info!("UpdateItem[{}]: Update operation completed.", key_owned);
    Ok(())
}
