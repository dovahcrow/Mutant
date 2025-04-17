pub mod cache;
pub mod error;
pub mod import;
pub mod integration_tests;
pub mod manager;
pub mod pool;
mod prepare;
pub mod verification;

pub use error::PadLifecycleError;
pub use manager::PadLifecycleManager;

pub(crate) use prepare::prepare_pads_for_store;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum PadOrigin {
    Generated,

    FreePool { initial_counter: u64 },
}
