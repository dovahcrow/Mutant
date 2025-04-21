// /// Defines error types specific to index operations.
// pub mod error;
// /// Defines the `IndexManager` trait and its default implementation for managing the index.
// pub mod manager;
// /// Handles the persistence (loading and saving) of the index data.
// pub mod persistence;
// /// Defines the core data structures representing the index (`MasterIndex`, `KeyInfo`, `PadInfo`).
pub mod error;
pub mod master_index;
pub mod pad_info;

/// Default size for scratchpads in bytes (4 MiB minus one page for metadata).
pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (4 * 1024 * 1024) - 4096;

pub(crate) use pad_info::{PadInfo, PadStatus};

#[cfg(test)]
mod integration_tests;
