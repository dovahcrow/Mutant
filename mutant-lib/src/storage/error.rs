use crate::network::NetworkError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Pad I/O operation failed: {0}")]
    PadIOError(String),

    #[error("Encryption operation failed: {0}")]
    EncryptionError(String),

    #[error("Decryption operation failed: {0}")]
    DecryptionError(String),

    #[error("Network layer error: {0}")]
    Network(#[from] NetworkError),

    #[error("Data transformation error: {0}")]
    TransformationError(String),
}
