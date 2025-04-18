pub mod cache;
pub mod error;
pub mod import;
pub mod integration_tests;
pub mod manager;
pub mod pool;
pub mod verification;

pub use error::PadLifecycleError;

pub(crate) mod prepare;
pub(crate) use prepare::prepare_pads_for_store;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum PadOrigin {
    Generated,

    FreePool { initial_counter: u64 },
}
