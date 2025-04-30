use crate::{index::error::IndexError, network::NetworkError};
use deadpool::managed::PoolError;
use never::Never;
use thiserror::Error;
use tokio::sync::AcquireError;

/// Represents the primary error type returned by `mutant-lib` functions.
///
/// This enum aggregates potential errors from various internal layers (network, storage, etc.)
/// as well as general operational errors.
#[derive(Error, Debug, Clone)]
pub enum Error {
    /// Errors related to configuration loading or validation.
    #[error("Configuration Error: {0}")]
    Config(String),

    /// Errors originating from the network layer (e.g., connection issues, peer discovery failures).
    #[error("Network Layer Error: {0}")]
    Network(#[from] NetworkError),

    #[error("Index Layer Error: {0}")]
    Index(#[from] IndexError),

    // /// Errors originating from the indexing layer (e.g., search failures, index inconsistency).
    // #[error("Index Layer Error: {0}")]
    // Index(#[from] IndexError),

    // /// Errors related to pad lifecycle management (e.g., creation, deletion, update failures).
    // #[error("Pad Lifecycle Layer Error: {0}")]
    // PadLifecycle(#[from] PadLifecycleError),

    // /// Errors related to data processing or handling (e.g., serialization, deserialization issues).
    // #[error("Data Operation Layer Error: {0}")]
    // Data(#[from] DataError),
    /// Errors occurring within user-provided callback functions.
    #[error("Callback Error: {0}")]
    Callback(String),

    /// Indicates that an operation was explicitly cancelled by a user callback.
    #[error("Operation cancelled by user callback")]
    OperationCancelled,

    /// Indicates that a requested feature or functionality is not yet implemented.
    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),

    /// Represents unexpected internal errors within the library.
    #[error("Internal Library Error: {0}")]
    Internal(String),

    /// Specific error indicating an operation was cancelled by a callback returning `false`.
    #[error("Callback cancelled operation")]
    CancelledByCallback,

    /// Specific error indicating a failure reported by a callback mechanism.
    #[error("Callback failed: {0}")]
    CallbackFailed(String),

    #[error("Callback error: {0}")]
    CallbackError(String),

    /// Errors related to the worker pool management (e.g., resource acquisition).
    #[error("Worker Pool Error: {0}")]
    PoolError(String),

    /// Indicates a timeout occurred during an operation.
    #[error("Operation timed out: {0}")]
    Timeout(String),
}

// Implementation to convert deadpool PoolError into our internal Error::PoolError
impl From<PoolError<Never>> for Error {
    fn from(e: PoolError<Never>) -> Self {
        Error::PoolError(e.to_string())
    }
}

// Implementation to convert tokio AcquireError into our internal Error::PoolError
impl From<AcquireError> for Error {
    fn from(e: AcquireError) -> Self {
        Error::PoolError(format!("Worker pool semaphore acquisition failed: {}", e))
    }
}
