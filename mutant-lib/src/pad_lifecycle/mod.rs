// Layer 3: Pad Lifecycle & Cache
pub mod cache;
pub mod error;
pub mod import;
pub mod manager;
pub mod pool;
pub mod verification;

pub use error::PadLifecycleError;
pub use manager::PadLifecycleManager; // Re-export the trait
