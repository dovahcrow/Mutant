// Layer 4: Data Operations
pub mod chunking;
pub mod error;
pub mod manager;
pub mod ops; // Declares the submodules store, fetch, remove, common

pub use error::DataError;
pub use manager::DataManager; // Re-export the trait
