# MutAnt Library: Internals

This document dives deeper into the internal data structures, component logic, and key algorithms used within the `mutant-lib`.

## 1. Core Data Structures (`mutant::data_structures`)

The heart of `mutant-lib`'s state management lies in these structures, which are serialized using CBOR and stored in the Master Index scratchpad.

### 1.1. `MasterIndexStorage`

```rust
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MasterIndexStorage {
    pub(crate) index: MasterIndex, // Alias for HashMap<String, KeyStorageInfo>
    #[serde(default)]
    pub(crate) free_pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Address and *Private Key* bytes
    pub(crate) scratchpad_size: usize,
}
```

*   `index`: A `HashMap` where keys are the user-provided strings and values are `KeyStorageInfo` structs detailing where the data resides.
*   `free_pads`: A `Vec` storing tuples of `(ScratchpadAddress, Vec<u8>)`. These represent data scratchpads that were previously used but whose data has been deleted (`MutAnt::remove`). The `Vec<u8>` contains the *private key* bytes needed to decrypt and reuse/overwrite the scratchpad. This allows for efficient pad recycling.
*   `scratchpad_size`: The usable size (in bytes) configured for data scratchpads (e.g., `PHYSICAL_SCRATCHPAD_SIZE - 512`). This value is determined during initialization and persisted here to ensure consistency.

### 1.2. `KeyStorageInfo`

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyStorageInfo {
    pub(crate) pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Address and *Public Key* bytes
    pub(crate) data_size: usize,
    #[serde(default = "Utc::now")]
    pub(crate) modified: DateTime<Utc>,
}
```

*   `pads`: A `Vec` storing tuples of `(ScratchpadAddress, Vec<u8>)`. For *each* scratchpad holding a chunk of the user key's data, this stores the pad's address and the corresponding *public key* bytes. The public key is sufficient for the network to locate the pad, but the private key (derived separately when needed) is required for decryption/updates.
*   `data_size`: The total size in bytes of the original user data associated with the key.
*   `modified`: The timestamp of the last `store` or `update` operation for this key.

## 2. Pad Manager Logic (`pad_manager`)

The `PadManager` orchestrates the use of scratchpads. It doesn't directly interact with the network but uses the `Storage` layer for that.

### 2.1. Write Operations (`pad_manager::write`)

*   **Goal:** Store potentially large `data_bytes` associated with a `user_key`.
*   **Steps:**
    1.  **Calculate Needs:** Determine the number of pads required based on `data_bytes.len()` and the `scratchpad_size` from the `MasterIndexStorage`.
    2.  **Allocate Pads:**
        *   Acquire a lock on the `MasterIndexStorage` cache.
        *   Check the `free_pads` list. Reuse pads from here if available, removing them from the list.
        *   If more pads are needed than available in `free_pads`, request new pads.
            *   Generate a new `SecretKey` (private key) for each required pad.
            *   Derive the corresponding `PublicKey` and `ScratchpadAddress`.
            *   Prepare tuples of `(ScratchpadAddress, private_key_bytes)` for creation and `(ScratchpadAddress, public_key_bytes)` for storing in `KeyStorageInfo`.
    3.  **Chunk Data:** Divide the `data_bytes` into chunks, each fitting within the `scratchpad_size`.
    4.  **Concurrent Writes (`pad_manager::write::concurrent`):**
        *   For each chunk and its corresponding allocated pad (address and *private key*):
            *   If the pad was from `free_pads`, instruct the `Storage` layer to *update* the existing scratchpad with the new chunk data using the provided private key.
            *   If it's a newly generated pad, instruct the `Storage` layer to *create* a new scratchpad with the chunk data using the generated private key.
        *   These create/update operations are spawned as concurrent tasks using `tokio::spawn` and `futures::future::join_all` to improve performance.
        *   Progress is reported via the optional `PutCallback`.
    5.  **Update Master Index:**
        *   Create a new `KeyStorageInfo` containing the list of `(ScratchpadAddress, public_key_bytes)` tuples for all pads used, the total `data_size`, and the current timestamp.
        *   Insert this into the `index` `HashMap` in the locked `MasterIndexStorage` cache, associating it with the `user_key`.
    6.  **Persist Master Index:** The `MutAnt::store` (or `update`) method, after the `PadManager` operation completes, calls `Storage::storage_save_mis_from_arc_static` to serialize the updated `MasterIndexStorage` cache and save it to its persistent scratchpad.

### 2.2. Read Operations (`pad_manager::read`)

*   **Goal:** Retrieve the full `data_bytes` associated with a `user_key`.
*   **Steps:**
    1.  **Lookup Key:** Acquire a lock on the `MasterIndexStorage` cache and find the `KeyStorageInfo` for the given `user_key` in the `index`.
    2.  **Identify Pads:** Get the list of `(ScratchpadAddress, public_key_bytes)` tuples from the `KeyStorageInfo.pads`.
    3.  **Derive Private Keys:** *Crucially*, the private key for each data pad is *not* stored in the `KeyStorageInfo`. It must be re-derived from the user's main `private_key_hex` (provided during `MutAnt::init`) combined with the `ScratchpadAddress`. (The exact derivation mechanism is within the `autonomi` crate, but the principle is deterministic derivation).
    4.  **Concurrent Fetches:**
        *   For each `ScratchpadAddress` and its derived *private key*:
            *   Instruct the `Storage` layer to fetch the raw (encrypted) data from the scratchpad using `Storage::fetch_scratchpad_internal_static`.
            *   The `Storage` layer handles decryption using the provided private key.
        *   These fetch operations are performed concurrently.
        *   Progress (bytes downloaded) is reported via the optional `GetCallback`.
    5.  **Reassemble Data:** Concatenate the decrypted byte chunks received from the `Storage` layer in the correct order.
    6.  **Verify Size:** Compare the size of the reassembled data with `KeyStorageInfo.data_size`. If they don't match, return an error (`Error::InternalError`).
    7.  Return the complete `Vec<u8>`.

### 2.3. Delete Operations (`pad_manager::delete`)

*   **Goal:** Remove the `user_key` and mark its associated pads as free.
*   **Steps:**
    1.  **Lookup and Remove:** Acquire a lock on the `MasterIndexStorage` cache. Find and remove the `KeyStorageInfo` entry for the `user_key` from the `index`.
    2.  **Recycle Pads:** If the key was found:
        *   Get the list of `(ScratchpadAddress, public_key_bytes)` associated with the removed key.
        *   For each pad, derive its *private key* (as done during reads).
        *   Add tuples of `(ScratchpadAddress, private_key_bytes)` to the `free_pads` list in the `MasterIndexStorage` cache.
    3.  **Persist Master Index:** `MutAnt::remove` saves the updated `MasterIndexStorage` cache.

## 3. Storage Layer Logic (`storage`)

The `Storage` layer is the bridge to the Autonomi network.

### 3.1. Initialization (`storage::init::new`)

*   Creates an `autonomi::Client` instance.
*   Determines the `MasterIndexAddress` and `MasterIndexKey` based on the provided user wallet/private key (likely using a deterministic derivation scheme).
*   Attempts to load the `MasterIndexStorage` from the network using `network::load_master_index_storage_static`.
    *   If found and deserializable, returns the `Arc<Mutex<MasterIndexStorage>>`.
    *   If not found (e.g., first run), creates a *new*, default `MasterIndexStorage` (empty index, empty free list, default scratchpad size) and saves it to the network using `network::storage_save_mis_from_arc_static`. Returns the `Arc<Mutex<>>` for this new instance.
*   Returns the `Storage` struct instance, the background task handle (if any), and the `Arc<Mutex<MasterIndexStorage>>`.

### 3.2. Network Operations (`storage::network`)

*   Wraps `autonomi::Client` calls (`scratchpad_get`, `scratchpad_update`, `scratchpad_create`).
*   Handles serialization to CBOR before writes (`storage_save_mis_from_arc_static`, `update_scratchpad_internal_static`) and deserialization after reads (`load_master_index_storage_static`).
*   Manages encryption/decryption using the provided `SecretKey` (private key) for each operation, often running decryption in `tokio::task::spawn_blocking` to avoid blocking the async runtime (`fetch_scratchpad_internal_static`).
*   Implements retry logic (`utils::retry::retry_operation`) around network calls to handle transient failures.
*   Translates `autonomi::Error` types into `mutant_lib::Error` types (e.g., `RecordNotFound` -> `Error::KeyNotFound`).

## 4. Error Handling (`error`)

The `error::Error` enum consolidates errors from various sources:

*   `IoError`: Underlying I/O issues.
*   `SerializationError`, `DeserializationError`: CBOR processing failures.
*   `AutonomiClient`: Errors originating from the `autonomi::Client`.
*   `AutonomiLibError`: Other errors from the `autonomi` crate (e.g., decryption).
*   `KeyNotFound`: Specific error when a user key or the Master Index isn't found.
*   `InvalidInput`: User provided invalid data (e.g., trying to use reserved keys).
*   `ConcurrencyError`: Issues with mutex locking.
*   `OperationNotSupported`: Attempting an invalid action (e.g., updating the Master Index directly).
*   `PadAllocationError`: Failure to allocate necessary scratchpads.
*   `InternalError`: Unexpected state or logic errors within the library.

## 5. Concurrency

*   The primary point of synchronization is the `MasterIndexStorage` cache, wrapped in `Arc<Mutex<>>`. This ensures that reads and modifications (like adding a key, removing a key, adding/removing free pads) are atomic and thread-safe.
*   Network operations (fetches, updates, creates) for *data* scratchpads are performed concurrently using `tokio::spawn` and `futures::future::join_all` within the `PadManager`'s read and write logic to maximize throughput.
*   Saving the Master Index itself is an atomic operation protected by the mutex.

## 6. Key Derivation (Conceptual)

While the exact implementation relies on `autonomi`, the ability to retrieve data requires deriving the correct private key for each data scratchpad. This is typically done using a Hierarchical Deterministic (HD) wallet-like scheme:

`PadPrivateKey = Derive(UserPrivateKey, PadAddress)`

This ensures that only the user possessing the initial `UserPrivateKey` can access their data, even though the `PadAddress` (derived from the pad's public key) is stored publicly in the Master Index. 