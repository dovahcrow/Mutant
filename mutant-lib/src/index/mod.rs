// Layer 2: Indexing & Metadata
pub mod error;
pub mod manager;
pub mod persistence;
pub mod query;
pub mod structure;

pub use error::IndexError;
pub use manager::IndexManager; // Re-export the trait
pub use structure::{KeyInfo, MasterIndex, PadInfo}; // Re-export core structures

#[cfg(test)]
pub mod integration_tests;
