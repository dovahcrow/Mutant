# MutAnt Library: Comprehensive Architecture & Implementation

This document provides an in-depth guide to the design, core data structures, component interactions, and operation flows within `mutant-lib`. It expands on both the high-level overview and the internal details to serve as a complete reference for users and contributors.

## 1. High-Level Overview

`mutant-lib` addresses the challenge of storing arbitrarily sized data blobs on the Autonomi network, which offers fixed-size scratchpads. It presents a cohesive key-value store abstraction with the following key concepts:

- **User Keys**: Human-friendly string identifiers for data blobs (e.g., `"config_v1"`).
- **Master Index**: A dedicated scratchpad storing a serialized `MasterIndexStorage`, mapping user keys to metadata and tracking free pads.
- **Data Scratchpads**: Regular scratchpads used to store encrypted chunks of user data.
- **Pad Manager**: Coordinates allocation, reuse, chunking, and verification of data scratchpads.
- **Storage Layer**: Low-level interface to the Autonomi network client (`autonomi::Client`), handling serialization, encryption, and network retries.

## 2. Architecture Diagram

```
Application  ──>  MutAnt API  ──>  Pad Manager  ──┐
                                                 │
                             ┌─────────────┐     │      ┌───────────────────┐
                             │             │     │      │                   │
Master Index Storage <── Mutex< MasterIndex > <──┘      │  Data Scratchpads  │
  (persistent)                     ^            mutex  │ (Autonomi Network)  │
                                   │                 └───────────────────┘
                             ┌─────────────┐               ^
                             │ Storage     │───────────────┘
                             │ Layer       │
                             └─────────────┘
```

## 3. Public API (`MutAnt`)

All operations are async and return `Result<_, Error>`:

- **Initialization**
  - `init(private_key_hex, config, callback)`
  - `init_with_progress(private_key_hex, config, callback)`

- **Data Operations**
  - `store(key: String, data: &[u8])`
  - `store_with_progress(key, data, callback)`
  - `fetch(key: &str) -> Vec<u8>`
  - `fetch_with_progress(key, callback)`
  - `remove(key: &str)`
  - `update(key: String, data: &[u8])` (unimplemented; use remove + store)

- **Inspection & Maintenance**
  - `list_keys() -> Vec<String>`
  - `list_key_details() -> Vec<KeyDetails>`
  - `get_storage_stats() -> StorageStats`
  - `import_free_pad(private_key_hex: &str)`
  - `reserve_pads(count: usize)`
  - `purge(callback)`

- **Index Management**
  - `save_master_index()`
  - `fetch_remote_master_index() -> MasterIndex`
  - `save_index_cache()`
  - `update_internal_master_index(index: MasterIndex)`
  - `get_index_copy() -> MasterIndex`

## 4. Layer Breakdown

### 4.1 Data Layer (`DataManager`)

- Defines `store`, `fetch`, `remove`, `update` via `async_trait`.
- Default implementation (`DefaultDataManager`) delegates to `data::ops`:
  - `store_op` handles chunking, pad lifecycle, network writes, index updates.
  - `fetch_op` handles index lookup, pad fetches, reassembly.
  - `remove_op` handles index removal and pad recycling.

### 4.2 Storage Layer (`StorageManager`)

- Wraps `autonomi::Client` for scratchpad operations:
  - `create_scratchpad`, `update_scratchpad`, `get_scratchpad`.
- Manages serialization (CBOR) and encryption/decryption.
- Implements retry logic for transient network failures.
- Loads and saves the `MasterIndexStorage` from/to its dedicated scratchpad.

### 4.3 Indexing Layer (`IndexManager`)

- Maintains `MasterIndex` in-memory and persists via storage layer.
- Provides query and persistence interfaces.
- Ensures index consistency and supports remote index creation prompts.

### 4.4 Network Layer (`NetworkAdapter`)

- Abstract trait for network operations:
  - Peer discovery, scratchpad reads/writes.
- Default Autonomi adapter (`AutonomiNetworkAdapter`) uses `autonomi::Client`.
- Handles address/key derivation, error translation.

### 4.5 Pad Lifecycle Layer (`PadLifecycleManager`)

- Manages free pad pool, allocation, preparation, and verification:
  - **Pool**: Tracks reusable pads harvested via `remove`.
  - **Prepare**: Generates new scratchpads when free pool is insufficient.
  - **Cache**: Persists index cache locally for performance.
  - **Import**: Imports external pads into free pool.
  - **Verification**: Confirms scratchpad writes before index update.

## 5. Core Data Structures

### 5.1 `MasterIndexStorage`

```rust
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MasterIndexStorage {
    pub(crate) index: MasterIndex,              // key -> KeyStorageInfo
    #[serde(default)]
    pub(crate) free_pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    pub(crate) scratchpad_size: usize,
}
```

- `index`: Maps each user key to its `KeyStorageInfo`.
- `free_pads`: Recycled scratchpads with private keys for reuse.
- `scratchpad_size`: Configured usable byte size per pad.

### 5.2 `KeyStorageInfo`

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyStorageInfo {
    pub(crate) pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    pub(crate) data_size: usize,
    #[serde(default = "Utc::now")]
    pub(crate) modified: DateTime<Utc>,
}
```

- `pads`: Addresses and public keys for each data chunk.
- `data_size`: Original data length.
- `modified`: Last update timestamp.

## 6. Operation Flows

### 6.1 Store

1. Determine chunk count from `data.len()` & `scratchpad_size`.
2. Lock `MasterIndexStorage`; extract free pads or prepare new ones.
3. Chunk data; spawn concurrent write tasks:
   - `update` existing pad or `create` new scratchpad.
   - Report progress via `PutCallback`.
4. Build `KeyStorageInfo` with pad addresses & public keys.
5. Insert into `index`; persist `MasterIndexStorage` via storage layer.

### 6.2 Fetch

1. Lock `MasterIndexStorage`; lookup `KeyStorageInfo`.
2. Derive per-pad private keys deterministically.
3. Spawn concurrent fetch tasks to read & decrypt chunks.
4. Reassemble chunks in order; verify total size matches.
5. Return full `Vec<u8>`; report progress via `GetCallback`.

### 6.3 Remove

1. Lock `MasterIndexStorage`; remove entry for key.
2. For each pad in `KeyStorageInfo`, derive private key.
3. Append `(address, private_key)` to `free_pads` list.
4. Persist updated `MasterIndexStorage`.

## 7. Error Model

Central `Error` enum unifies:

- Configuration errors
- Network, Storage, Index, PadLifecycle, Data errors (via `#[from]`)
- Callback cancellations & failures
- `NotImplemented`, `Internal` cases

## 8. Concurrency & Thread Safety

- The `MasterIndexStorage` is wrapped in `Arc<Mutex<>>`, ensuring atomic updates.
- Data scratchpad operations run in parallel via `tokio::spawn` + `join_all`.
- Storage layer uses `spawn_blocking` for CPU-bound decryption.

## 9. Encryption & Key Derivation

- Master index private key is derived from user key hex via SHA-256 → `SecretKey`.
- Pad private keys for new pads are randomly generated; for recycled pads loaded from `free_pads`.
- Data pad keys are not stored in index; they are re-derived or reused for updates.

## 10. Extensibility & Customization

- Trait-based abstractions for each layer:
  - `NetworkAdapter`, `StorageManager`, `IndexManager`, `PadLifecycleManager`, `DataManager`.
- Users can implement custom backends by satisfying these traits.
- `MutAntConfig` allows selecting alternate network or tuning scratchpad size.

## 11. Further Reading

- `overview.md` for a concise introduction.
- `internals.md` for deep dives into algorithms and test coverage.
- Crate API docs on https://docs.rs/mutant_lib 