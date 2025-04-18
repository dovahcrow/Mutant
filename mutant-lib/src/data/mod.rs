/// Provides utilities for splitting data into fixed-size chunks.
pub mod chunking;
/// Defines error types specific to data operations.
pub mod error;
mod integration_tests;
/// Defines the `DataManager` trait and its default implementation.
pub mod manager;
/// Contains implementations for core data operations (store, fetch, remove, update).
pub(crate) mod ops;

/// Re-exports the primary error type for the data module.
pub use error::DataError;

/// Data encoding type for the master index scratchpad.
pub(crate) const MASTER_INDEX_ENCODING: u64 = 1;
/// Data encoding type for private data chunk scratchpads.
pub(crate) const PRIVATE_DATA_ENCODING: u64 = 2;
/// Data encoding type for public data chunk scratchpads.
pub(crate) const PUBLIC_DATA_ENCODING: u64 = 3;
/// Data encoding type for the public index scratchpad.
pub(crate) const PUBLIC_INDEX_ENCODING: u64 = 4;
