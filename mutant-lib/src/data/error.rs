use crate::index::IndexError;
use crate::pad_lifecycle::PadLifecycleError;
use crate::storage::StorageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DataError {
    #[error("Data chunking failed: {0}")]
    ChunkingError(String),

    #[error("Data reassembly failed: {0}")]
    ReassemblyError(String),

    #[error("Insufficient free pads available for operation: {0}")]
    InsufficientFreePads(String),

    #[error("Failed to update data: {0}")]
    DataUpdateError(String),

    #[error("Key not found for operation: {0}")]
    KeyNotFound(String),

    #[error("Key already exists: {0}. Use --force to overwrite.")]
    KeyAlreadyExists(String),

    #[error("Index layer error during data operation: {0}")]
    Index(#[from] IndexError),

    #[error("Pad lifecycle layer error during data operation: {0}")]
    PadLifecycle(#[from] PadLifecycleError),

    #[error("Storage layer error during data operation: {0}")]
    Storage(#[from] StorageError),

    #[error("Operation cancelled by callback")]
    OperationCancelled,

    #[error("Internal data operation error: {0}")]
    InternalError(String),

    #[error("Inconsistent state detected: {0}")]
    InconsistentState(String),

    #[error("Cryptography error: {0}")]
    CryptoError(String),
}

impl From<blsttc::Error> for DataError {
    fn from(err: blsttc::Error) -> Self {
        DataError::CryptoError(format!("BLSTTC error: {}", err))
    }
}
