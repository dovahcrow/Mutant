use crate::storage::StorageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Serialization failed: {0}")]
    SerializationError(String),

    #[error("Deserialization failed: {0}")]
    DeserializationError(String),

    #[error("Key not found in index: {0}")]
    KeyNotFound(String),

    #[error("Index persistence operation failed: {0}")]
    IndexPersistenceError(String),

    #[error("Storage layer error during index operation: {0}")]
    Storage(#[from] StorageError), // Propagate storage errors

    #[error("Inconsistent index state: {0}")]
    InconsistentState(String),

    #[error("Internal index error: {0}")]
    InternalError(String),
    // Add other specific index-related errors as needed
}

// Helper for converting serde_cbor errors
impl From<serde_cbor::Error> for IndexError {
    fn from(err: serde_cbor::Error) -> Self {
        // Check if it's an EOF error, which might indicate an empty or non-existent index
        if err.is_eof() {
            IndexError::DeserializationError(
                "End of file reached unexpectedly (potentially empty index data)".to_string(),
            )
        } else {
            IndexError::DeserializationError(format!("CBOR error: {}", err))
        }
    }
}
