# Internals: Storage Layer

*(Note: The specific module might be named `storage`, `network`, or similar in the source code.)*

The Storage Layer acts as the bridge between `mutant-lib`'s higher-level logic (like the Pad Manager) and the underlying Autonomi network client (`autonomi::Client`). Its primary role is to execute specific scratchpad operations requested by other parts of the library, handling the necessary details like serialization, encryption, and network communication.

## Key Responsibilities

1.  **Network Client Interaction:** Directly calling methods on the `autonomi::Client` instance (e.g., `scratchpad_create`, `scratchpad_update`, `scratchpad_get`).
2.  **Serialization/Deserialization:** Converting the `MasterIndexStorage` struct to/from CBOR format before writing to or after reading from its dedicated scratchpad.
3.  **Encryption/Decryption:** Encrypting data chunks before writing them to data scratchpads and decrypting them after retrieval, using the per-pad private keys provided by the caller (usually the Pad Manager).
4.  **Master Index Persistence:** Loading the `MasterIndexStorage` during `MutAnt::init` and saving it back whenever requested (e.g., after `store` or `remove`).
5.  **Retry Logic:** Implementing strategies (e.g., exponential backoff) to automatically retry failed network operations that might be transient.
6.  **Error Translation:** Converting errors from the `autonomi::Client` or other underlying operations (like serialization) into the library's unified `mutant_lib::error::Error` type.

## Operations

### Master Index Handling

*   **Loading (`load_master_index_storage`):**
    *   Called during `MutAnt::init`.
    *   Calculates the Master Index address and key based on the user's private key.
    *   Calls `autonomi::Client::scratchpad_get` with the derived address and key.
    *   If successful, decrypts the fetched bytes using the derived key.
    *   Deserializes the decrypted bytes from CBOR into a `MasterIndexStorage` struct.
    *   If the scratchpad is not found (`autonomi::Error::RecordNotFound`), it signals that a new index needs to be created.
    *   Returns the loaded (or indicates the need for a new) `MasterIndexStorage`.
*   **Saving (`save_master_index_storage`):**
    *   Called after operations that modify the index cache (e.g., `store`, `remove`, `reserve_pads`).
    *   Takes the current `MasterIndexStorage` instance (usually from the `Arc<Mutex<>>` cache).
    *   Serializes the struct into CBOR bytes.
    *   Derives the Master Index address and key.
    *   Calls `autonomi::Client::scratchpad_update` (or potentially `scratchpad_create` if it was newly created) with the address, key, and serialized CBOR data.
    *   Applies retry logic around the network call.

### Data Scratchpad Handling

*   **Creating (`create_scratchpad`):**
    *   Called by the Pad Manager when a *new* data pad needs to be populated.
    *   Receives the target `ScratchpadAddress`, the `SecretKey` (private key) for *this specific pad*, and the raw data `chunk`.
    *   Encrypts the `chunk` using the provided `SecretKey`.
    *   Calls `autonomi::Client::scratchpad_create` with the address, the *public key* derived from the secret key, and the encrypted chunk.
    *   Applies retry logic.
*   **Updating (`update_scratchpad`):**
    *   Called by the Pad Manager when *reusing* a pad from the `free_pads` list.
    *   Receives the target `ScratchpadAddress`, the `SecretKey` (retrieved from `free_pads`), and the raw data `chunk`.
    *   Encrypts the `chunk` using the provided `SecretKey`.
    *   Calls `autonomi::Client::scratchpad_update` with the address, the `SecretKey` (required for update authorization), and the encrypted chunk.
    *   Applies retry logic.
*   **Fetching (`fetch_scratchpad`):**
    *   Called by the Pad Manager during a `fetch` operation for each required data chunk.
    *   Receives the `ScratchpadAddress` to fetch from and the corresponding `SecretKey` (deterministically derived by the Pad Manager).
    *   Calls `autonomi::Client::scratchpad_get` with the address and the `SecretKey`.
    *   Applies retry logic.
    *   If successful, decrypts the fetched encrypted bytes using the provided `SecretKey`.
        *   Decryption can be CPU-intensive, so it's often performed in a `tokio::task::spawn_blocking` context to avoid blocking the async runtime.
    *   Returns the decrypted `Vec<u8>` chunk.

## Implementation Details

*   **Client Instance:** Typically holds an instance of `autonomi::Client` (or a trait object representing it for testability).
*   **Configuration:** May hold relevant parts of the `MutAntConfig`, such as retry parameters.
*   **Asynchronous:** All network operations are asynchronous, leveraging `async`/`await` and potentially `tokio` for task management (especially for decryption).

By encapsulating the direct network interactions, serialization, and encryption details, the Storage Layer provides a cleaner interface for the rest of the library to work with persistent storage. 