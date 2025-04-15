use crate::network::NetworkError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Pad I/O operation failed: {0}")]
    PadIOError(String),

    #[error("Encryption operation failed: {0}")]
    EncryptionError(String), // Placeholder if encryption is added

    #[error("Decryption operation failed: {0}")]
    DecryptionError(String), // Placeholder if encryption is added

    #[error("Network layer error: {0}")]
    Network(#[from] NetworkError), // Propagate network errors

    #[error("Data transformation error: {0}")]
    TransformationError(String), // For checksums or other transformations

                                 // Add other specific storage-related errors as needed
}
