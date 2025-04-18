use crate::internal_error::Error as LibError;

use autonomi::ScratchpadAddress;
use std::future::Future;
use std::pin::Pin;

/// Contains initialization logic for the MutAnt API.
pub mod init;
/// Defines the main `MutAnt` struct and its associated methods.
pub mod mutant;

/// Re-exports the main `MutAnt` structure for easy access.
pub use mutant::MutAnt;

use crate::internal_events::PutEvent;
/// Callback type specific to the API layer for `put` operations.
/// Note: This might shadow the callback type defined in `crate::events`.
#[allow(dead_code)]
pub type PutCallback = Box<
    dyn Fn(PutEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

use crate::internal_events::GetEvent;
/// Callback type specific to the API layer for `get` operations.
/// Note: This might shadow the callback type defined in `crate::events`.
#[allow(dead_code)]
pub type GetCallback = Box<
    dyn Fn(GetEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>> + Send + Sync,
>;

/// Events emitted during a `reserve` operation (explicit pad reservation).
#[derive(Debug, Clone)]
pub enum ReserveEvent {
    /// Indicates the start of the reserve operation.
    Starting {
        /// The total number of pads requested.
        total_requested: usize,
    },
    /// Indicates that a single pad has been successfully reserved.
    PadReserved {
        /// The address of the reserved pad.
        address: ScratchpadAddress,
    },
    /// Indicates that the index is being updated with the newly reserved pads.
    SavingIndex {
        /// The number of pads successfully reserved so far.
        reserved_count: usize,
    },
    /// Indicates the completion of the reserve operation.
    Complete {
        /// The total number of pads successfully reserved.
        succeeded: usize,
        /// The number of pads that failed to be reserved.
        failed: usize,
    },
}

/// Callback type used during `reserve` operations to report progress and allow cancellation.
///
/// The callback receives `ReserveEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type ReserveCallback = Box<
    dyn Fn(ReserveEvent) -> Pin<Box<dyn Future<Output = Result<bool, LibError>> + Send>>
        + Send
        + Sync,
>;
