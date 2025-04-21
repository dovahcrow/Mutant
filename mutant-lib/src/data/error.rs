use crate::index::IndexError;
use crate::network::NetworkError;
use crate::pad_lifecycle::PadLifecycleError;
use autonomi::ScratchpadAddress;
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

    /// An operation was attempted that is invalid in the current context (e.g., updating a private key as public).
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

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

    /// Failed to serialize data (e.g., list of chunk addresses for public index).
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Failed to deserialize data (e.g., list of chunk addresses from public index).
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// A fetched public index or data scratchpad had an invalid signature.
    #[error("Invalid signature for scratchpad {0}")]
    InvalidSignature(ScratchpadAddress),

    /// A fetched public index scratchpad had an incorrect data encoding type.
    #[error("Invalid public index encoding (expected 4, got {0})")]
    InvalidPublicIndexEncoding(u64),

    /// A fetched public data scratchpad had an incorrect data encoding type.
    #[error("Invalid public data encoding (expected 3, got {0})")]
    InvalidPublicDataEncoding(u64),

    /// Failed to find a public scratchpad (index or data) at the specified address.
    #[error("Public scratchpad not found at address {0}")]
    PublicScratchpadNotFound(ScratchpadAddress),

    /// An error occurred within a user-provided callback.
    #[error("Callback error: {0}")]
    CallbackError(String),

    /// A required payment receipt was missing for scratchpad {0}
    #[error("A required payment receipt was missing for scratchpad {0}")]
    MissingReceipt(ScratchpadAddress),

    /// Confirmation failed: Fetched data length did not match expected length for {address}.
    #[error("Confirmation data mismatch for pad {address}: expected {expected}, got {actual}")]
    ConfirmationDataMismatch {
        address: ScratchpadAddress,
        expected: usize,
        actual: usize,
    },

    /// Confirmation failed: Timed out waiting for pad {0} to be retrievable.
    #[error("Confirmation timed out for pad {0}")]
    ConfirmationTimeout(ScratchpadAddress),
}

impl From<blsttc::Error> for DataError {
    fn from(err: blsttc::Error) -> Self {
        DataError::CryptoError(format!("BLSTTC error: {}", err))
    }
}

impl From<serde_cbor::Error> for DataError {
    fn from(err: serde_cbor::Error) -> Self {
        DataError::Deserialization(err.to_string())
    }
}
