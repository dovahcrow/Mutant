# MutAnt Library: API Reference

This document details the public API provided by the `mutant_lib::MutAnt` struct.

All methods are asynchronous and typically return `Result<T, mutant_lib::error::Error>`.

## Initialization

These methods create and configure a `MutAnt` instance.

### `MutAnt::init`

```rust
pub async fn init(private_key_hex: &str) -> Result<Self, Error>
```

Initializes the `MutAnt` instance with the given Autonomi private key (hex-encoded string) for the Mainnet network.

* Connects to the Autonomi network.
* Derives the Master Index address and key from `private_key_hex`.
* Attempts to load the `MasterIndex` from the network.
* If the index is not found, creates a new, empty one.
* Returns a `MutAnt` instance ready for use.

**Arguments:**

* `private_key_hex`: Your Autonomi wallet's private key as a hex string.

**Returns:** `Result<MutAnt, Error>`

### `MutAnt::init_public`

```rust
pub async fn init_public() -> Result<Self, Error>
```

Initializes a `MutAnt` instance for public fetching only. This instance can only be used to fetch public data using `get_public`.

**Returns:** `Result<MutAnt, Error>`

### `MutAnt::init_local`

```rust
pub async fn init_local() -> Result<Self, Error>
```

Initializes a `MutAnt` instance for local development using the Devnet network.

**Returns:** `Result<MutAnt, Error>`

### `MutAnt::init_alphanet`

```rust
pub async fn init_alphanet(private_key_hex: &str) -> Result<Self, Error>
```

Initializes a `MutAnt` instance for the Alphanet network.

**Arguments:**

* `private_key_hex`: Your Autonomi wallet's private key as a hex string.

**Returns:** `Result<MutAnt, Error>`

## Data Operations

Methods for storing, retrieving, and removing data.

### `MutAnt::put`

```rust
pub async fn put(
    &self,
    user_key: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>
) -> Result<ScratchpadAddress, Error>
```

Stores the given `data_bytes` under the specified `user_key`.

* Handles data chunking, pad allocation (reuse or creation), encryption, and concurrent writes to data scratchpads.
* Updates the `MasterIndex` with the new key and metadata.
* If the `user_key` already exists with the same data, resumes the operation.
* If the `user_key` already exists with different data, updates the key.

**Arguments:**

* `user_key`: The string key to associate with the data.
* `data_bytes`: An `Arc<Vec<u8>>` containing the data to store.
* `mode`: The `StorageMode` to use (Small, Medium, Large).
* `public`: Whether to make the data publicly accessible.
* `no_verify`: Whether to skip verification of written pads.
* `put_callback`: An optional callback function to report progress.

**Returns:** `Result<ScratchpadAddress, Error>` - The address of the first pad (or index pad for public keys).

### `MutAnt::get`

```rust
pub async fn get(
    &self,
    user_key: &str,
    get_callback: Option<GetCallback>
) -> Result<Vec<u8>, Error>
```

Retrieves the data associated with the given `user_key`.

* Looks up the `user_key` in the `MasterIndex`.
* Fetches encrypted data chunks concurrently from the relevant data scratchpads.
* Decrypts and reassembles the chunks in the correct order.

**Arguments:**

* `user_key`: The string key of the data to retrieve.
* `get_callback`: An optional callback function to report progress.

**Returns:** `Result<Vec<u8>, Error>`. Returns `Error::KeyNotFound` if the key does not exist.

### `MutAnt::get_public`

```rust
pub async fn get_public(
    &self,
    address: &ScratchpadAddress,
    get_callback: Option<GetCallback>
) -> Result<Vec<u8>, Error>
```

Retrieves publicly accessible data using its public address.

* Fetches the index pad at the given address.
* Extracts the addresses of the data pads from the index pad.
* Fetches and reassembles the data pads.

**Arguments:**

* `address`: The `ScratchpadAddress` of the public index pad.
* `get_callback`: An optional callback function to report progress.

**Returns:** `Result<Vec<u8>, Error>`.

### `MutAnt::rm`

```rust
pub async fn rm(&self, user_key: &str) -> Result<(), Error>
```

Removes the data and metadata associated with the given `user_key`.

* Removes the `user_key` entry from the `MasterIndex`.
* Moves the associated pads to the `free_pads` list in the `MasterIndex` for later reuse.
* Note: This does *not* immediately delete or overwrite the data on the data scratchpads, it just makes them available for reuse.

**Arguments:**

* `user_key`: The string key of the data to remove.

**Returns:** `Result<(), Error>`.

## Inspection & Maintenance

Methods for querying the state of stored data and managing resources.

### `MutAnt::list`

```rust
pub async fn list(&self) -> Result<BTreeMap<String, IndexEntry>, Error>
```

Returns a map of all keys and their associated index entries currently stored in the Master Index.

**Returns:** `Result<BTreeMap<String, IndexEntry>, Error>`

### `MutAnt::contains_key`

```rust
pub async fn contains_key(&self, user_key: &str) -> bool
```

Checks if the given key exists in the Master Index.

**Arguments:**

* `user_key`: The string key to check.

**Returns:** `bool` - `true` if the key exists, `false` otherwise.

### `MutAnt::get_public_index_address`

```rust
pub async fn get_public_index_address(&self, user_key: &str) -> Result<String, Error>
```

Gets the public address for a public key. This address can be used with `get_public` to fetch the data without a private key.

**Arguments:**

* `user_key`: The string key to get the public address for.

**Returns:** `Result<String, Error>` - The hex-encoded public address.

### `MutAnt::get_storage_stats`

```rust
pub async fn get_storage_stats(&self) -> StorageStats
```

Provides statistics about storage usage, such as the number of keys, total pads, occupied pads, and free pads.

**Returns:** `StorageStats`

### `MutAnt::purge`

```rust
pub async fn purge(
    &self,
    aggressive: bool,
    purge_callback: Option<PurgeCallback>
) -> Result<PurgeResult, Error>
```

Verifies and cleans up invalid pads.

**Arguments:**

* `aggressive`: Whether to check all pads or only those that need verification.
* `purge_callback`: An optional callback function to report progress.

**Returns:** `Result<PurgeResult, Error>`

### `MutAnt::health_check`

```rust
pub async fn health_check(
    &self,
    key_name: &str,
    recycle: bool,
    health_check_callback: Option<HealthCheckCallback>
) -> Result<HealthCheckResult, Error>
```

Checks the health of a specific key by verifying all its pads.

**Arguments:**

* `key_name`: The string key to check.
* `recycle`: Whether to recycle invalid pads.
* `health_check_callback`: An optional callback function to report progress.

**Returns:** `Result<HealthCheckResult, Error>`

### `MutAnt::sync`

```rust
pub async fn sync(
    &self,
    force: bool,
    sync_callback: Option<SyncCallback>
) -> Result<SyncResult, Error>
```

Synchronizes the local index with the remote index.

**Arguments:**

* `force`: Whether to force synchronization even if the local index is up to date.
* `sync_callback`: An optional callback function to report progress.

**Returns:** `Result<SyncResult, Error>`

## Import/Export

Methods for importing and exporting pad private keys.

### `MutAnt::export_raw_pads_private_key`

```rust
pub async fn export_raw_pads_private_key(&self) -> Result<Vec<PadInfo>, Error>
```

Exports the private keys of all pads in the Master Index.

**Returns:** `Result<Vec<PadInfo>, Error>`

### `MutAnt::import_raw_pads_private_key`

```rust
pub async fn import_raw_pads_private_key(&self, pads_hex: Vec<PadInfo>) -> Result<(), Error>
```

Imports pad private keys into the Master Index.

**Arguments:**

* `pads_hex`: A vector of `PadInfo` structs containing the pad information to import.

**Returns:** `Result<(), Error>`

## Storage Modes

The `StorageMode` enum defines the chunk size used for storing data:

```rust
pub enum StorageMode {
    Small,   // 128KB chunks
    Medium,  // 2MB chunks
    Large,   // 8MB chunks
}
```

## Callback Types

The library provides several callback types for progress reporting:

* `PutCallback`: Reports progress of put operations.
* `GetCallback`: Reports progress of get operations.
* `PurgeCallback`: Reports progress of purge operations.
* `HealthCheckCallback`: Reports progress of health check operations.
* `SyncCallback`: Reports progress of sync operations.

## Error Types

The `Error` enum defines the possible errors that can occur:

* `KeyNotFound`: The requested key was not found.
* `NetworkError`: An error occurred in the network layer.
* `IndexError`: An error occurred in the index layer.
* `Internal`: An internal error occurred.
* `Config`: An error occurred in the configuration.
* `PoolError`: An error occurred in the worker pool.

## Data Structures

### `PadInfo`

```rust
pub struct PadInfo {
    pub address: ScratchpadAddress,
    pub private_key: Vec<u8>,
    pub status: PadStatus,
    pub size: usize,
    pub checksum: u64,
    pub last_known_counter: u64,
}
```

### `PadStatus`

```rust
pub enum PadStatus {
    Generated,
    Written,
    Confirmed,
    Free,
    Invalid,
}
```

### `IndexEntry`

```rust
pub enum IndexEntry {
    PrivateKey(Vec<PadInfo>),
    PublicUpload(PadInfo, Vec<PadInfo>),
}
```

### `StorageStats`

```rust
pub struct StorageStats {
    pub nb_keys: u64,
    pub total_pads: u64,
    pub occupied_pads: u64,
    pub free_pads: u64,
    pub pending_verification_pads: u64,
}
```