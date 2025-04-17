use crate::data::error::DataError;
use crate::data::ops;
use crate::data::ops::common::DataManagerDependencies;
use crate::events::{GetCallback, PutCallback};
use crate::index::IndexManager;
use crate::network::NetworkAdapter;
use crate::pad_lifecycle::PadLifecycleManager;
use crate::storage::StorageManager;
use async_trait::async_trait;
use std::sync::Arc;

/// Trait defining the core data operations: store, fetch, remove, update.
///
/// Implementations of this trait manage the high-level logic for interacting with data,
/// coordinating between indexing, pad lifecycle, storage, and networking layers.
#[async_trait]
pub trait DataManager: Send + Sync {
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
    async fn store(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), DataError>;

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
    async fn fetch(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, DataError>;

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
    async fn remove(&self, user_key: &str) -> Result<(), DataError>;

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
    async fn update(
        &self,
        _user_key: String,
        _data_bytes: &[u8],
        _callback: Option<PutCallback>,
    ) -> Result<(), DataError>;
}

/// Default implementation of the `DataManager` trait.
///
/// This struct holds references (via `Arc`) to the necessary dependencies (index manager,
/// pad lifecycle manager, storage manager, network adapter) and delegates the core
/// data operations (`store`, `fetch`, `remove`) to specific functions within the `ops` module.
pub struct DefaultDataManager {
    deps: DataManagerDependencies, // Internal field holding dependencies
}

impl DefaultDataManager {
    /// Creates a new instance of `DefaultDataManager`.
    ///
    /// # Arguments
    ///
    /// * `index_manager` - An `Arc` reference to an `IndexManager` implementation.
    /// * `pad_lifecycle_manager` - An `Arc` reference to a `PadLifecycleManager` implementation.
    /// * `storage_manager` - An `Arc` reference to a `StorageManager` implementation.
    /// * `network_adapter` - An `Arc` reference to a `NetworkAdapter` implementation.
    ///
    /// # Returns
    ///
    /// A new `DefaultDataManager` instance.
    pub fn new(
        index_manager: Arc<dyn IndexManager>,
        pad_lifecycle_manager: Arc<dyn PadLifecycleManager>,
        storage_manager: Arc<dyn StorageManager>,
        network_adapter: Arc<dyn NetworkAdapter>,
    ) -> Self {
        Self {
            deps: DataManagerDependencies {
                index_manager,
                pad_lifecycle_manager,
                storage_manager,
                network_adapter,
            },
        }
    }
}

#[async_trait]
impl DataManager for DefaultDataManager {
    async fn store(
        &self,
        user_key: String,
        data_bytes: &[u8],
        callback: Option<PutCallback>,
    ) -> Result<(), DataError> {
        ops::store::store_op(&self.deps, user_key, data_bytes, callback).await
    }

    async fn fetch(
        &self,
        user_key: &str,
        callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, DataError> {
        ops::fetch::fetch_op(&self.deps, user_key, callback).await
    }

    async fn remove(&self, user_key: &str) -> Result<(), DataError> {
        ops::remove::remove_op(&self.deps, user_key).await
    }

    async fn update(
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
