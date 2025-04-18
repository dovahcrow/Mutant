use crate::data::DataError;
use crate::index::IndexError;
use crate::network::NetworkError;
use crate::pad_lifecycle::PadLifecycleError;
use crate::storage::StorageError;
use thiserror::Error;

/// Represents the primary error type returned by `mutant-lib` functions.
///
/// This enum aggregates potential errors from various internal layers (network, storage, etc.)
/// as well as general operational errors.
#[derive(Error, Debug)]
pub enum Error {
    /// Errors related to configuration loading or validation.
    #[error("Configuration Error: {0}")]
    Config(String),

    /// Errors originating from the network layer (e.g., connection issues, peer discovery failures).
    #[error("Network Layer Error: {0}")]
    Network(#[from] NetworkError),

    /// Errors originating from the storage backend (e.g., I/O errors, data corruption).
    #[error("Storage Layer Error: {0}")]
    Storage(#[from] StorageError),

    /// Errors originating from the indexing layer (e.g., search failures, index inconsistency).
    #[error("Index Layer Error: {0}")]
    Index(#[from] IndexError),

    /// Errors related to pad lifecycle management (e.g., creation, deletion, update failures).
    #[error("Pad Lifecycle Layer Error: {0}")]
    PadLifecycle(#[from] PadLifecycleError),

    /// Errors related to data processing or handling (e.g., serialization, deserialization issues).
    #[error("Data Operation Layer Error: {0}")]
    Data(#[from] DataError),

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
}
