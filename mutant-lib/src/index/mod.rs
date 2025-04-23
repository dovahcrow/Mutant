pub mod error;
pub mod master_index;
pub mod pad_info;

/// Default size for scratchpads in bytes (4 MiB minus one page for metadata).
// pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (4 * 1024 * 1024) - 4096;
pub(crate) const DEFAULT_SCRATCHPAD_SIZE: usize = (2 * 1024 * 1024);

pub(crate) use pad_info::{PadInfo, PadStatus};

#[cfg(test)]
mod integration_tests;
