use crate::index::IndexError;
use crate::network::NetworkError;
use crate::pad_lifecycle::PadLifecycleError;
use thiserror::Error;

/// Represents errors that can occur during data management operations (store, fetch, remove, update).
#[derive(Error, Debug)]
pub enum DataError {
    /// Error occurred during the process of splitting data into chunks.
    #[error("Data chunking failed: {0}")]
    ChunkingError(String),

    /// Error occurred during the process of reassembling chunks into original data.
    #[error("Data reassembly failed: {0}")]
    ReassemblyError(String),

    /// Not enough free storage pads were available to complete the requested operation.
    #[error("Insufficient free pads available for operation: {0}")]
    InsufficientFreePads(String),

    /// An error occurred specifically during an update operation.
    #[error("Failed to update data: {0}")]
    DataUpdateError(String),

    /// The specified key could not be found in the index.
    #[error("Key not found for operation: {0}")]
    KeyNotFound(String),

    /// An attempt was made to store data with a key that already exists.
    #[error("Key already exists: {0}. Use --force to overwrite.")]
    KeyAlreadyExists(String),

    /// An error propagated from the index layer.
    #[error("Index layer error during data operation: {0}")]
    Index(#[from] IndexError),

    /// An error propagated from the pad lifecycle management layer.
    #[error("Pad lifecycle layer error during data operation: {0}")]
    PadLifecycle(#[from] PadLifecycleError),

    /// An error propagated from the network layer.
    #[error("Network layer error during data operation: {0}")]
    Network(#[from] NetworkError),

    /// The operation was cancelled by a user-provided callback returning `false`.
    #[error("Operation cancelled by callback")]
    OperationCancelled,

    /// An unexpected internal error occurred within the data management logic.
    #[error("Internal data operation error: {0}")]
    InternalError(String),

    /// An inconsistent state was detected, suggesting a potential bug or data corruption.
    #[error("Inconsistent state detected: {0}")]
    InconsistentState(String),

    /// An error occurred during cryptographic operations (e.g., encryption, decryption, signing).
    #[error("Cryptography error: {0}")]
    CryptoError(String),
}

impl From<blsttc::Error> for DataError {
    fn from(err: blsttc::Error) -> Self {
        DataError::CryptoError(format!("BLSTTC error: {}", err))
    }
}
