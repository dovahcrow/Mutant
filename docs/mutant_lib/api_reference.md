# MutAnt Library: API Reference

This document details the public API provided by the `mutant_lib::api::MutAnt` struct.

All methods are asynchronous and typically return `Result<T, mutant_lib::error::Error>`.

## Initialization

These methods create and configure a `MutAnt` instance.

### `MutAnt::init`

```rust
pub async fn init(
    private_key_hex: String,
    config: MutAntConfig
) -> Result<Self, Error>
```

Initializes the `MutAnt` instance with the given Autonomi private key (hex-encoded string) and configuration.

*   Connects to the Autonomi network.
*   Derives the Master Index address and key from `private_key_hex`.
*   Attempts to load the `MasterIndexStorage` from the network.
*   If the index is not found, creates a new, empty one and saves it to the network.
*   Returns a `MutAnt` instance ready for use.

**Arguments:**

*   `private_key_hex`: Your Autonomi wallet's private key as a hex string.
*   `config`: A `MutAntConfig` struct specifying operational parameters (e.g., scratchpad size, network settings, retry behavior). Use `MutAntConfig::default()` for standard settings.

**Returns:** `Result<MutAnt, Error>`

### `MutAnt::init_with_progress`

```rust
pub async fn init_with_progress(
    private_key_hex: String,
    config: MutAntConfig,
    callback: Option<InitCallback>
) -> Result<Self, Error>
```

Same as `init`, but accepts an optional callback function (`InitCallback`) to report the progress of the initialization steps.

**Arguments:**

*   `private_key_hex`: Your Autonomi wallet's private key as a hex string.
*   `config`: A `MutAntConfig` struct.
*   `callback`: An `Option<InitCallback>` (typically `Option<Arc<Mutex<dyn FnMut(InitEvent) + Send + Sync>>>`). See `mutant_lib::events` for `InitEvent` details.

**Returns:** `Result<MutAnt, Error>`

## Data Operations

Methods for storing, retrieving, and removing data.

### `MutAnt::store`

```rust
pub async fn store(&self, key: &str, data: &[u8]) -> Result<(), Error>
```

Stores the given `data` bytes under the specified `key`.

*   Handles data chunking, pad allocation (reuse or creation), encryption, and concurrent writes to data scratchpads.
*   Updates the `MasterIndexStorage` with the new key and metadata.
*   Saves the updated `MasterIndexStorage` to the network.
*   If the `key` already exists, this method will likely fail (use `update` or `remove`+`store`).

**Arguments:**

*   `key`: The string key to associate with the data.
*   `data`: A byte slice (`&[u8]`) containing the data to store.

**Returns:** `Result<(), Error>`

### `MutAnt::store_with_progress`

```rust
pub async fn store_with_progress(
    &self,
    key: &str,
    data: &[u8],
    callback: Option<ProgressCallback>
) -> Result<(), Error>
```

Same as `store`, but accepts an optional `ProgressCallback` to report the progress of chunk uploads and retries.

**Arguments:**

*   `key`: The string key.
*   `data`: The byte slice data.
*   `callback`: An `Option<ProgressCallback>`. See `mutant_lib::events` for `ProgressEvent` details.

**Returns:** `Result<(), Error>`

### `MutAnt::fetch`

```rust
pub async fn fetch(&self, key: &str) -> Result<Vec<u8>, Error>
```

Retrieves the data associated with the given `key`.

*   Looks up the `key` in the `MasterIndexStorage`.
*   Fetches encrypted data chunks concurrently from the relevant data scratchpads.
*   Decrypts and reassembles the chunks in the correct order.
*   Verifies the total data size.

**Arguments:**

*   `key`: The string key of the data to retrieve.

**Returns:** `Result<Vec<u8>, Error>`. Returns `Error::KeyNotFound` if the key does not exist.

### `MutAnt::fetch_with_progress`

```rust
pub async fn fetch_with_progress(
    &self,
    key: &str,
    callback: Option<ProgressCallback>
) -> Result<Vec<u8>, Error>
```

Same as `fetch`, but accepts an optional `ProgressCallback` to report the progress of chunk downloads and retries.

**Arguments:**

*   `key`: The string key.
*   `callback`: An `Option<ProgressCallback>`.

**Returns:** `Result<Vec<u8>, Error>`.

### `MutAnt::remove`

```rust
pub async fn remove(&self, key: &str) -> Result<(), Error>
```

Removes the data and metadata associated with the given `key`.

*   Removes the `key` entry from the `MasterIndexStorage`.
*   Derives the private keys for the associated data pads.
*   Adds the data pads (address and private key) to the `free_pads` list in the `MasterIndexStorage` for later reuse.
*   Saves the updated `MasterIndexStorage`.
*   Note: This does *not* immediately delete or overwrite the data on the data scratchpads, it just makes them available for reuse.

**Arguments:**

*   `key`: The string key of the data to remove.

**Returns:** `Result<(), Error>`.

### `MutAnt::update` (Conceptual / Potential)

*(Note: Based on `architecture.md`, a direct `update` might not be implemented yet. The current recommended approach is `remove` followed by `store`.)*

```rust
// Conceptual signature
// pub async fn update(&self, key: &str, data: &[u8]) -> Result<(), Error>
```

Atomically updates the data associated with an existing key. This is often more complex than `remove` + `store` as it might involve resizing, changing pad allocation, and ensuring atomicity.

**Current Recommendation:** Use `mutant.remove(key).await?` followed by `mutant.store(key, new_data).await?`.

## Inspection & Maintenance

Methods for querying the state of stored data and managing resources.

### `MutAnt::list_keys`

```rust
pub async fn list_keys(&self) -> Result<Vec<String>, Error>
```

Returns a list of all keys currently stored in the Master Index.

**Returns:** `Result<Vec<String>, Error>`

### `MutAnt::list_key_details`

```rust
pub async fn list_key_details(&self) -> Result<Vec<KeyDetails>, Error>
```

Returns a list of `KeyDetails` structs, providing more information about each stored key (like size and modification time). See `mutant_lib::types::KeyDetails`.

**Returns:** `Result<Vec<KeyDetails>, Error>`

### `MutAnt::get_storage_stats`

```rust
pub async fn get_storage_stats(&self) -> Result<StorageStats, Error>
```

Provides statistics about storage usage, such as the number of keys, total data size, number of used pads, and number of free pads. See `mutant_lib::types::StorageStats`.

**Returns:** `Result<StorageStats, Error>`

### `MutAnt::reserve_pads`

```rust
pub async fn reserve_pads(&self, count: usize) -> Result<(), Error>
```

Pre-allocates a specified number of new scratchpads and adds them to the `free_pads` list in the Master Index. This can potentially speed up future `store` operations if you anticipate needing many new pads soon.

**Arguments:**

*   `count`: The number of new scratchpads to create and reserve.

**Returns:** `Result<(), Error>`

*(Other maintenance methods like `import_free_pad` and `purge` might exist - refer to source or specific documentation if needed)*

## Index Management

Advanced methods for interacting directly with the Master Index cache or remote storage. Use with caution.

### `MutAnt::save_master_index`

```rust
pub async fn save_master_index(&self) -> Result<(), Error>
```

Forces the current in-memory `MasterIndexStorage` cache to be serialized and saved to its dedicated scratchpad on the network.

**Returns:** `Result<(), Error>`

*(Other index methods like `fetch_remote_master_index`, `save_index_cache`, `update_internal_master_index`, `get_index_copy` might exist for advanced use cases or debugging - refer to source or specific documentation if needed)* 