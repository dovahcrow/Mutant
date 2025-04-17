// Layer 1: Storage Abstraction
pub mod error;
pub mod manager;
pub mod pad_io;

pub use error::StorageError;
pub use manager::StorageManager; // Re-export the trait

#[cfg(test)]
pub mod integration_tests;
