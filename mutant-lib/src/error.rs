// SPDX-License-Identifier: Apache-2.0

use autonomi::client::data_types::scratchpad::ScratchpadError;
// use std::error::Error as StdError; // Unused
use thiserror::Error;
use tokio::task::JoinError;

/// Errors that can occur within the mutant-lib.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Network initialization failed: {0}")]
    NetworkInitError(String),
    #[error("Wallet creation failed: {0}")]
    WalletError(String),
    #[error("Storage operation failed: {0}")]
    StorageError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    #[error("Key not found: {0}")]
    KeyNotFound(String),
    #[error("Key already exists: {0}")]
    KeyAlreadyExists(String),
    #[error("Data too large: {0}")]
    DataTooLarge(String),
    #[error("Scratchpad creation failed: {0}")]
    CreationFailed(String),
    #[error("Scratchpad fetch failed: {0}")]
    FetchFailed(String),
    #[error("Scratchpad update failed: {0}")]
    UpdateFailed(String),
    #[error("Scratchpad remove failed: {0}")]
    RemoveFailed(String),
    #[error("Direct storage failed: {0}")]
    DirectStoreFailed(String),
    #[error("Direct fetch failed: {0}")]
    DirectFetchFailed(String),
    #[error("Chunk storage failed: {0}")]
    ChunkStoreFailed(String),
    #[error("Failed to fetch chunk {chunk_index} ({address}) for key '{key}': {source}")]
    ChunkFetchFailed {
        key: String,
        chunk_index: usize,
        address: autonomi::ScratchpadAddress,
        source: Box<Error>, // Box the underlying error
    },
    #[error("Invalid internal state: {0}")]
    InternalError(String),
    #[error("Operation not supported")]
    OperationNotSupported,
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Scratchpad fetch failed: {0}")]
    ScratchpadFetchFailed(String),
    #[error("Scratchpad read failed: {0}")]
    ScratchpadReadFailed(String),
    #[error("Invalid read range requested for scratchpad: {0}")]
    InvalidReadRange(String),
    #[error("Allocation failed: {0}")]
    AllocationFailed(String),
    #[error("Deallocation failed: {0}")]
    DeallocationFailed(String),
    #[error("Insufficient allocated space: {0}")]
    InsufficientSpace(String),
    #[error("Operation cancelled by user or callback")]
    OperationCancelled,
    #[error("Failed to upload data")]
    FailedToUploadData,
    #[error("Failed to retrieve data")]
    FailedToRetrieveData,
    #[error("Failed to acquire lock")]
    LockError,
    #[error("Failed to connect to network: {0}")]
    NetworkConnectionFailed(String),
    #[error("Failed to create wallet: {0}")]
    WalletCreationFailed(String),
    #[error("Failed to derive vault key: {0}")]
    VaultKeyDerivationFailed(String),
    #[error("Failed to fetch from vault: {0}")]
    VaultFetchFailed(String),
    #[error("Failed to store to vault: {0}")]
    VaultStoreFailed(String),
    #[error("Failed to initialize storage")]
    StorageInitializationFailed,
    #[error("Pack management error: {0}")]
    PackManagementError(String),
    #[error("Internal key not found within data pack")]
    ItemNotInPack,
    #[error("Feature not implemented: {0}")]
    NotImplemented(String),
    #[error("Invalid data location found in index")]
    InvalidDataLocation,
    #[error("CBOR serialization/deserialization error: {0}")]
    Cbor(#[from] serde_cbor::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Autonomi scratchpad client error: {0}")]
    AutonomiClient(#[from] ScratchpadError),
    #[error("Task join error: {0}")]
    TaskJoinError(String),
    #[error("Vault write failed: {0}")]
    VaultWriteFailed(String),
    #[error("Scratchpad get failed: {0}")]
    GetFailed(String),
    #[error("Scratchpad delete failed: {0}")]
    DeleteFailed(String),
    #[error("Allocator error: {0}")]
    AllocatorError(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error(
        "Reconstructed data for key '{key}' is incomplete: expected {expected} bytes, got {actual}"
    )]
    IncompleteData {
        key: String,
        expected: u64,
        actual: u64,
    },
    #[error("Dialoguer interaction error: {0}")]
    DialoguerError(String),
    #[error("Operation cancelled by user")]
    UserCancelled,
    #[error("Autonomi library error: {0}")]
    AutonomiLibError(String),
    #[error("Operation timed out: {0}")]
    Timeout(String),
    #[error("Tokio task join error: {0}")]
    JoinError(String),
    #[error("Verification failed after timeout: {0}")]
    VerificationTimeout(String),
}

impl Error {
    pub fn from_join_error_msg(join_error: &JoinError, context_msg: String) -> Self {
        let cause = if join_error.is_panic() {
            "Task panicked".to_string()
        } else if join_error.is_cancelled() {
            "Task cancelled".to_string()
        } else {
            "Unknown task failure".to_string()
        };
        Error::InternalError(format!("{}: {} ({})", context_msg, cause, join_error))
    }
}

impl From<JoinError> for Error {
    fn from(err: JoinError) -> Self {
        Error::JoinError(err.to_string())
    }
}
