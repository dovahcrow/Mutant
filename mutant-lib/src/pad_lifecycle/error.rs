use crate::index::IndexError;
use crate::network::NetworkError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PadLifecycleError {
    #[error("Failed to read index from local cache: {0}")]
    CacheReadError(String),

    #[error("Failed to write index to local cache: {0}")]
    CacheWriteError(String),

    #[error("Pad verification process failed: {0}")]
    VerificationFailed(String),

    #[error("Pad import failed due to conflict: {0}")]
    ImportConflict(String),

    #[error("Failed to generate or acquire new pads: {0}")]
    PadAcquisitionFailed(String),

    #[error("Index layer error during pad lifecycle operation: {0}")]
    Index(#[from] IndexError),

    #[error("Network layer error during pad lifecycle operation: {0}")]
    Network(#[from] NetworkError),

    #[error("Invalid input for pad lifecycle operation: {0}")]
    InvalidInput(String),

    #[error("Operation cancelled by callback")]
    OperationCancelled,

    #[error("Internal pad lifecycle error: {0}")]
    InternalError(String),
}

impl From<std::io::Error> for PadLifecycleError {
    fn from(err: std::io::Error) -> Self {
        PadLifecycleError::CacheReadError(format!("IO error: {}", err))
    }
}
