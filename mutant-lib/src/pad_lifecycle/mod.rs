// Layer 3: Pad Lifecycle & Cache
pub mod cache;
pub mod error;
pub mod import;
pub mod manager;
pub mod pool;
pub mod verification;

pub use error::PadLifecycleError;
pub use manager::PadLifecycleManager; // Re-export the trait

use serde::{Deserialize, Serialize};

/// Indicates whether a pad was newly generated or reused from the free pool.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum PadOrigin {
    /// Pad was newly generated.
    Generated,
    /// Pad was taken from the existing free pool.
    FreePool { initial_counter: u64 },
}
