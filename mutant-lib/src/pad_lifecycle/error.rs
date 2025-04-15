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
    VerificationFailed(String), // General verification failure

    #[error("Pad import failed due to conflict: {0}")]
    ImportConflict(String), // e.g., pad already exists

    #[error("Failed to generate or acquire new pads: {0}")]
    PadAcquisitionFailed(String), // If pad generation is added later or acquisition fails

    #[error("Index layer error during pad lifecycle operation: {0}")]
    Index(#[from] IndexError), // Propagate index errors

    #[error("Network layer error during pad lifecycle operation: {0}")]
    Network(#[from] NetworkError), // Propagate network errors (e.g., during verification)

    #[error("Invalid input for pad lifecycle operation: {0}")]
    InvalidInput(String), // e.g., invalid private key hex for import

    #[error("Operation cancelled by callback")]
    OperationCancelled,

    #[error("Internal pad lifecycle error: {0}")]
    InternalError(String),
    // Add other specific lifecycle-related errors as needed
}

// Helper for IO errors during cache operations
impl From<std::io::Error> for PadLifecycleError {
    fn from(err: std::io::Error) -> Self {
        // Determine if it's more likely a read or write error based on context?
        // For simplicity, map to a generic cache error for now.
        PadLifecycleError::CacheReadError(format!("IO error: {}", err)) // Or CacheWriteError
    }
}
