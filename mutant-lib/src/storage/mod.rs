pub mod error;
pub mod manager;
pub mod pad_io;

pub use error::StorageError;
pub use manager::StorageManager;

#[cfg(test)]
pub mod integration_tests;
