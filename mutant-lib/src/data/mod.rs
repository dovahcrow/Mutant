// Layer 4: Data Operations
pub mod chunking;
pub mod error;
pub mod manager;
pub mod ops; // Contains the implementation logic for store, fetch, etc.

pub use error::DataError;
pub use manager::DataManager; // Re-export the trait
