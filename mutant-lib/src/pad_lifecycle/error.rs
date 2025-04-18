use crate::index::IndexError;
use crate::network::NetworkError;
use thiserror::Error;

/// Errors specific to the pad lifecycle management module.
#[derive(Error, Debug)]
pub enum PadLifecycleError {
    /// Error reading the local index cache file.
    #[error("Failed to read index from local cache: {0}")]
    CacheReadError(String),

    /// Error writing to the local index cache file.
    #[error("Failed to write index to local cache: {0}")]
    CacheWriteError(String),

    /// Error during the pad verification process (checking existence on network).
    #[error("Pad verification process failed: {0}")]
    VerificationFailed(String),

    /// Error importing an external pad, often due to index conflicts.
    #[error("Pad import failed due to conflict: {0}")]
    ImportConflict(String),

    /// Error acquiring pads, either from the free pool or by generating new ones.
    #[error("Failed to generate or acquire new pads: {0}")]
    PadAcquisitionFailed(String),

    /// An error originating from the index management layer.
    #[error("Index layer error during pad lifecycle operation: {0}")]
    Index(#[from] IndexError),

    /// An error originating from the network layer.
    #[error("Network layer error during pad lifecycle operation: {0}")]
    Network(#[from] NetworkError),

    /// Input provided to a lifecycle operation was invalid (e.g., bad key format).
    #[error("Invalid input for pad lifecycle operation: {0}")]
    InvalidInput(String),

    /// The operation was explicitly cancelled by a user-provided callback.
    #[error("Operation cancelled by callback")]
    OperationCancelled,

    /// An unexpected internal error occurred within the pad lifecycle module.
    #[error("Internal pad lifecycle error: {0}")]
    InternalError(String),
}

impl From<std::io::Error> for PadLifecycleError {
    fn from(err: std::io::Error) -> Self {
        PadLifecycleError::CacheReadError(format!("IO error: {}", err))
    }
}
