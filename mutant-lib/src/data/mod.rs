// Layer 4: Data Operations
pub mod chunking;
pub mod error;
mod integration_tests;
pub mod manager;
pub mod ops; // Declares the submodules store, fetch, remove, common

pub use error::DataError;
pub use manager::DataManager; // Re-export the trait
