// Layer 5: Public API
// This module defines the public MutAnt struct and its methods, acting as the
// main entry point for the library. It orchestrates the underlying managers.

use crate::error::Error as LibError;
// use crate::types::MutAntConfig; // Removed unused import
use autonomi::ScratchpadAddress;
use std::future::Future;
use std::pin::Pin;

pub mod init;
pub mod mutant; // Keep MutAnt struct and impls in a separate file

pub use mutant::MutAnt; // Re-export the main struct
                        // Potentially re-export config/types if they aren't exposed at the top level lib.rs
                        // pub use crate::types::{MutAntConfig, KeyDetails, StorageStats};
                        // pub use crate::events::*; // Re-export events if needed by API users directly

// Callback types for Put operations
use crate::events::PutEvent;
pub type PutCallback = Box<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

// Callback types for Get operations
use crate::events::GetEvent;
pub type GetCallback = Box<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

// Callback types for Reserve operations
#[derive(Debug, Clone)]
pub enum ReserveEvent {
    Starting { total_requested: usize },
    PadReserved { address: ScratchpadAddress },
    SavingIndex { reserved_count: usize },
    Complete { succeeded: usize, failed: usize },
}

pub type ReserveCallback = Box<
    dyn Fn(ReserveEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>>
        + Send
        + Sync,
>;

pub use crate::events; // Re-export events module
