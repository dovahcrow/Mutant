use crate::network::NetworkError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Serialization failed: {0}")]
    SerializationError(String),

    #[error("Deserialization failed: {0}")]
    DeserializationError(String),

    #[error("Failed to decrypt index data: {0}")]
    DecryptionError(String),

    #[error("Key not found in index: {0}")]
    KeyNotFound(String),

    #[error("Index persistence operation failed: {0}")]
    IndexPersistenceError(String),

    #[error("Network layer error during index operation: {0}")]
    Network(#[from] NetworkError),

    #[error("Inconsistent index state: {0}")]
    InconsistentState(String),

    #[error("Internal index error: {0}")]
    InternalError(String),
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
