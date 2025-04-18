/// Defines error types specific to index operations.
pub mod error;
/// Defines the `IndexManager` trait and its default implementation for managing the index.
pub mod manager;
/// Handles the persistence (loading and saving) of the index data.
pub mod persistence;
/// Provides functions for querying and calculating statistics from the index.
pub mod query;
/// Defines the core data structures representing the index (`MasterIndex`, `KeyInfo`, `PadInfo`).
pub mod structure;

/// Re-exports the primary error type for the index module.
pub use error::IndexError;
/// Re-exports the core index data structures.
pub use structure::{KeyInfo, PadInfo};

#[cfg(test)]
mod integration_tests; // Renamed to mod as it's test-only
