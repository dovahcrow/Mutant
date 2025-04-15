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
    DataUpdateError(String), // Specific errors during update logic

    #[error("Key not found for operation: {0}")]
    KeyNotFound(String), // If an operation requires a key that doesn't exist

    #[error("Key already exists: {0}. Use --force to overwrite.")]
    KeyAlreadyExists(String), // Attempted to store a key that already exists without force

    #[error("Index layer error during data operation: {0}")]
    Index(#[from] IndexError), // Propagate index errors

    #[error("Pad lifecycle layer error during data operation: {0}")]
    PadLifecycle(#[from] PadLifecycleError), // Propagate lifecycle errors

    #[error("Storage layer error during data operation: {0}")]
    Storage(#[from] StorageError), // Propagate storage errors

    #[error("Operation cancelled by callback")]
    OperationCancelled,

    #[error("Internal data operation error: {0}")]
    InternalError(String),
    // Add other specific data operation errors as needed
}
