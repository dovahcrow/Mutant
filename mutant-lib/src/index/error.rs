use thiserror::Error;

use crate::network::NetworkChoice;

#[derive(Error, Debug, PartialEq, Clone)]
pub enum IndexError {
    #[error("Key already exists: {0}")]
    KeyAlreadyExists(String),

    #[error("Key not found: {0}")]
    KeyNotFound(String),

    #[error("Cannot remove public upload key: {0}")]
    CannotRemovePublicUpload(String),

    #[error("Index file not found: {0}")]
    IndexFileNotFound(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Network mismatch: {x:?} != {y:?}")]
    NetworkMismatch { x: NetworkChoice, y: NetworkChoice },
}
