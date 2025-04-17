/// Provides utilities for splitting data into fixed-size chunks.
pub mod chunking;
/// Defines error types specific to data operations.
pub mod error;
mod integration_tests;
/// Defines the `DataManager` trait and its default implementation.
pub mod manager;
/// Contains implementations for core data operations (store, fetch, remove, update).
pub mod ops;

/// Re-exports the primary error type for the data module.
pub use error::DataError;
