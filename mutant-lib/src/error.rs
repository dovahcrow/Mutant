 // Might need if API layer has specific errors later
use crate::data::DataError;
use crate::index::IndexError;
use crate::network::NetworkError;
use crate::pad_lifecycle::PadLifecycleError;
use crate::storage::StorageError;
use thiserror::Error;

/// Top-level error enum for the MutAnt library.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration Error: {0}")]
    Config(String),

    #[error("Network Layer Error: {0}")]
    Network(#[from] NetworkError),

    #[error("Storage Layer Error: {0}")]
    Storage(#[from] StorageError),

    #[error("Index Layer Error: {0}")]
    Index(#[from] IndexError),

    #[error("Pad Lifecycle Layer Error: {0}")]
    PadLifecycle(#[from] PadLifecycleError),

    #[error("Data Operation Layer Error: {0}")]
    Data(#[from] DataError),

    // Errors originating from callbacks
    #[error("Callback Error: {0}")]
    Callback(String), // Generic callback error for now

    // Specific high-level errors
    #[error("Operation cancelled by user callback")]
    OperationCancelled,

    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),

    #[error("Internal Library Error: {0}")]
    Internal(String),
    // Add other top-level specific errors as needed
}

// Optional: Add From implementations for specific callback-related errors if needed
// e.g., impl From<CallbackSpecificError> for Error { ... }
