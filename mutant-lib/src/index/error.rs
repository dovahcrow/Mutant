use crate::network::NetworkError;
use autonomi::ScratchpadAddress;
use thiserror::Error;

/// Errors that can occur within the index management module.
#[derive(Error, Debug)]
pub enum IndexError {
    /// Error during CBOR serialization of index data.
    #[error("Serialization failed: {0}")]
    SerializationError(String),

    /// Error during CBOR deserialization of index data.
    #[error("Deserialization failed: {0}")]
    DeserializationError(String),

    /// Error decrypting index data retrieved from the network.
    #[error("Failed to decrypt index data: {0}")]
    DecryptionError(String),

    /// The requested key was not found in the master index.
    #[error("Key not found in index: {0}")]
    KeyNotFound(String),

    /// An error occurred while trying to persist the index to the network.
    #[error("Index persistence operation failed: {0}")]
    IndexPersistenceError(String),

    /// An error originating from the network layer during an index operation.
    #[error("Network layer error during index operation: {0}")]
    Network(#[from] NetworkError),

    /// The index state is inconsistent (e.g., unexpected pad status).
    #[error("Inconsistent index state: {0}")]
    InconsistentState(String),

    /// An unexpected internal error occurred within the index module.
    #[error("Internal index error: {0}")]
    InternalError(String),

    /// Key '{0}' already exists in the index
    #[error("Key '{0}' already exists in the index")]
    KeyExists(String),

    /// Public upload name '{0}' already exists
    #[error("Public upload name '{0}' already exists")]
    PublicUploadNameExists(String),

    /// Pad with address '{0}' not found for key '{1}'
    #[error("Pad with address '{0}' not found for key '{1}'")]
    PadNotFound(autonomi::ScratchpadAddress, String),

    /// Public upload '{0}' not found
    #[error("Public upload '{0}' not found")]
    PublicUploadNotFound(String),

    /// Entry '{0}' is not a public upload
    #[error("Entry '{0}' is not a public upload")]
    NotAPublicUpload(String),

    /// Data pad with index {1} not found for public upload '{0}'
    #[error("Data pad with index {1} not found for public upload '{0}'")]
    DataPadNotFound(String, usize),

    /// Could not acquire lock
    #[error("Could not acquire lock")]
    Lock,

    /// Pad lifecycle error: {0}
    #[error("Pad lifecycle error: {0}")]
    PadLifecycle(String),
}

impl From<serde_cbor::Error> for IndexError {
    fn from(err: serde_cbor::Error) -> Self {
        if err.is_eof() {
            IndexError::DeserializationError(
                "End of file reached unexpectedly (potentially empty index data)".to_string(),
            )
        } else {
            IndexError::DeserializationError(format!("CBOR error: {}", err))
        }
    }
}
