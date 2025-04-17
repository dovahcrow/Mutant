use crate::data::DataError;
use crate::index::IndexError;
use crate::network::NetworkError;
use crate::pad_lifecycle::PadLifecycleError;
use crate::storage::StorageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration Error: {0}")]
    Config(String),

    #[error("Network Layer Error: {0}")]
    Network(#[from] NetworkError),

    #[error("Storage Layer Error: {0}")]
    Storage(#[from] StorageError),

    #[error("Index Layer Error: {0}")]
    Index(#[from] IndexError),

    #[error("Pad Lifecycle Layer Error: {0}")]
    PadLifecycle(#[from] PadLifecycleError),

    #[error("Data Operation Layer Error: {0}")]
    Data(#[from] DataError),

    #[error("Callback Error: {0}")]
    Callback(String),

    #[error("Operation cancelled by user callback")]
    OperationCancelled,

    #[error("Feature not yet implemented: {0}")]
    NotImplemented(String),

    #[error("Internal Library Error: {0}")]
    Internal(String),

    #[error("Callback cancelled operation")]
    CancelledByCallback,

    #[error("Callback failed: {0}")]
    CallbackFailed(String),
}
