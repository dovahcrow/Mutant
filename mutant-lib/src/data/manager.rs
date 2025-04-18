use crate::data::error::DataError;
use crate::data::ops;
use crate::index::manager::DefaultIndexManager;
use crate::internal_events::{GetCallback, PutCallback};
use crate::network::AutonomiNetworkAdapter;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use log::trace;
use std::sync::Arc;

/// Default implementation of the `DataManager` trait.
///
/// This struct holds references (via `Arc`) to the necessary dependencies (index manager,
/// pad lifecycle manager, storage manager, network adapter) and delegates the core
/// data operations (`store`, `fetch`, `remove`) to specific functions within the `ops` module.
pub struct DefaultDataManager {
    pub(crate) network_adapter: Arc<AutonomiNetworkAdapter>,
    pub(crate) index_manager: Arc<DefaultIndexManager>,
    pub(crate) pad_lifecycle_manager: Arc<DefaultPadLifecycleManager>,
}

impl DefaultDataManager {
    /// Creates a new instance of `DefaultDataManager`.
    ///
    /// # Arguments
    ///
    /// * `network_adapter` - An `Arc` reference to a `AutonomiNetworkAdapter` implementation.
    /// * `index_manager` - An `Arc` reference to an `IndexManager` implementation.
    /// * `pad_lifecycle_manager` - An `Arc` reference to a `PadLifecycleManager` implementation.
    ///
    /// # Returns
    ///
    /// A new `DefaultDataManager` instance.
    pub fn new(
        network_adapter: Arc<AutonomiNetworkAdapter>,
        index_manager: Arc<DefaultIndexManager>,
        pad_lifecycle_manager: Arc<DefaultPadLifecycleManager>,
    ) -> Self {
        trace!("Initializing DefaultDataManager");
        Self {
            network_adapter,
            index_manager,
            pad_lifecycle_manager,
        }
    }

    /// Stores the given data bytes under the specified user key.
    ///
    /// This involves chunking the data, finding/reserving storage pads, writing chunks,
    /// confirming writes, and updating the index entry for the key.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The unique identifier for the data.
    /// * `data_bytes` - The raw data to store.
    /// * `callback` - An optional callback for progress reporting and cancellation.
    ///
    /// # Errors
    ///
    /// Returns a `DataError` if any step of the storage process fails.
    pub async fn store(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), DataError> {
        ops::store::store_op(self, user_key, data_bytes, callback).await
    }

    /// Fetches the data associated with the given user key.
    ///
    /// This involves looking up the key in the index, retrieving the locations of data chunks,
    /// fetching the chunks from storage, and reassembling them.
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key identifying the data to fetch.
    /// * `callback` - An optional callback for progress reporting and cancellation.
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the reassembled data.
    ///
    /// # Errors
    ///
    /// Returns `DataError::NotFound` if the key does not exist, or other `DataError` variants
    /// if fetching or reassembly fails.
    pub async fn fetch(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, DataError> {
        ops::fetch::fetch_op(self, user_key, callback).await
    }

    /// Removes the data associated with the given user key.
    ///
    /// This involves removing the key entry from the index and potentially marking
    /// the associated storage pads as available for reuse (depending on implementation and purge strategy).
    ///
    /// # Arguments
    ///
    /// * `user_key` - The key identifying the data to remove.
    ///
    /// # Errors
    ///
    /// Returns `DataError::NotFound` if the key does not exist, or other `DataError` variants
    /// if the removal process fails.
    pub async fn remove(&self, user_key: &str) -> Result<(), DataError> {
        ops::remove::remove_op(self, user_key).await
    }

    /// Updates the data for an existing key.
    ///
    /// **Note:** This operation is currently unimplemented and will panic.
    /// Use `remove` followed by `store` as a workaround.
    ///
    /// # Arguments
    ///
    /// * `_user_key` - The key identifying the data to update.
    /// * `_data_bytes` - The new data bytes.
    /// * `_callback` - An optional callback for progress reporting.
    ///
    /// # Errors
    ///
    /// Currently panics.
    pub async fn update(
        &self,
        _user_key: String,
        _data_bytes: &[u8],
        _callback: Option<PutCallback>,
    ) -> Result<(), DataError> {
        unimplemented!(
            "Update operation is temporarily removed due to complexity. Use remove then store."
        );
    }
}
