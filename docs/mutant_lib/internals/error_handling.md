# Internals: Error Handling

`mutant-lib` aims to provide a clear and unified way to report errors that can occur during its operations. It defines a central error enum, `mutant_lib::error::Error`, which encompasses issues arising from network interactions, data processing, configuration, and internal logic.

## The `Error` Enum

The primary error type is defined in `src/error.rs` (or similar). It typically looks something like this (specific variants may differ based on the exact implementation):

```rust
use thiserror::Error;
use autonomi::common::PublicKey;
// Add other necessary imports, e.g., for CBOR errors, IO errors, etc.

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Key not found: {0}")]
    KeyNotFound(String), // The key that wasn't found

    #[error("Master index not found for user: {0}")]
    MasterIndexNotFound(PublicKey), // Public key of the user

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("CBOR serialization error: {0}")]
    SerializationError(String), // Simplified, might wrap specific CBOR error type

    #[error("CBOR deserialization error: {0}")]
    DeserializationError(String), // Simplified, might wrap specific CBOR error type

    #[error("Autonomi client error: {0}")]
    AutonomiClient(#[from] autonomi::Error), // Errors from the autonomi crate

    #[error("Cryptography error: {0}")]
    CryptoError(String), // e.g., key derivation failure, decryption failure
    
    #[error("Concurrency error (e.g., mutex poisoning): {0}")]
    ConcurrencyError(String),

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Failed to allocate necessary scratchpads: {0}")]
    PadAllocationError(String),

    #[error("Operation cancelled by callback")]
    OperationCancelled,

    #[error("Feature not implemented: {0}")]
    NotImplemented(String),

    #[error("Internal library error: {0}")]
    InternalError(String),
    
    // ... other specific variants as needed ...
}
```

## Key Error Variants

*   **`ConfigError`:** Issues with the provided `MutAntConfig` during initialization.
*   **`InvalidInput`:** User provided invalid arguments, such as an improperly formatted key or trying to use a reserved key name.
*   **`KeyNotFound`:** The requested key does not exist in the Master Index during a `fetch` or `remove` operation.
*   **`MasterIndexNotFound`:** The library could not load or create the Master Index scratchpad during `init` (potentially a network or permission issue).
*   **`IoError`:** Underlying standard library I/O errors, often related to local caching if implemented.
*   **`SerializationError` / `DeserializationError`:** Failures during CBOR processing of the `MasterIndexStorage` or potentially other internal data.
*   **`AutonomiClient`:** Wraps errors originating directly from the `autonomi::Client` library. This includes network issues (`NetworkError`), record not found (`RecordNotFound`), authentication problems, etc. Using `#[from]` allows easy conversion.
*   **`CryptoError`:** Failures related to cryptographic operations like key derivation or data decryption.
*   **`ConcurrencyError`:** Issues related to mutex locking, such as a poisoned mutex (indicating a panic while a lock was held).
*   **`TimeoutError`:** An operation (like a network request with retry) exceeded its allowed time.
*   **`PadAllocationError`:** The Pad Manager failed to secure enough scratchpads (either free or new) for a `store` operation.
*   **`OperationCancelled`:** A progress callback indicated that the operation should be halted.
*   **`NotImplemented`:** A called feature or method path is not yet implemented.
*   **`InternalError`:** Catches unexpected states or logic bugs within the library (e.g., data size mismatch after reassembly during `fetch`).

## Error Propagation and Handling

*   **`thiserror`:** The library likely uses the `thiserror` crate to easily implement `std::error::Error` and provide descriptive messages.
*   **`#[from]`:** Where appropriate, `#[from]` is used to automatically convert errors from underlying libraries (like `std::io::Error` or `autonomi::Error`) into the corresponding `mutant_lib::error::Error` variant. This simplifies error handling within the library.
*   **`Result<T, Error>`:** Most public API functions return `Result<T, mutant_lib::error::Error>`, requiring the calling application to handle potential failures.
*   **Retry Logic:** The Storage Layer often incorporates retry mechanisms (e.g., using `backoff` or a custom implementation) for network errors wrapped within `AutonomiClient`. These retries happen internally before the error is propagated upwards if the operation ultimately fails.

By consolidating diverse error sources into a single enum, `mutant-lib` provides a consistent error handling experience for its users. 