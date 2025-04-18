# MutAnt Library: Comprehensive Architecture & Implementation

This document provides an in-depth guide to the design, core data structures, component interactions, and operation flows within `mutant-lib`. It expands on both the high-level overview and the internal details to serve as a complete reference for users and contributors.

## 1. High-Level Overview

`mutant-lib` addresses the challenge of storing arbitrarily sized data blobs on the Autonomi network, which offers fixed-size scratchpads. It presents a cohesive key-value store abstraction with the following key concepts:

- **User Keys**: Human-friendly string identifiers for data blobs (e.g., `"config_v1"`).
- **Master Index**: A dedicated scratchpad storing a serialized `MasterIndex`, mapping user keys to metadata and tracking free pads.
- **Data Scratchpads**: Regular scratchpads used to store encrypted chunks of user data.
- **Pad Manager**: Coordinates allocation, reuse, chunking, and verification of data scratchpads.
- **Network Layer**: Low-level interface to the Autonomi network client (`autonomi::Client`), handling serialization, encryption, and network retries for scratchpad persistence.

## 2. Architecture Diagram

```
Application  ──>  MutAnt API  ──>  Pad Manager  ────────────> Index Manager ──┐
                                                                               │
                                   ┌───────────────────┐    ┌───────────────────┐
                                   │ Mutex<MasterIndex>│<───┤      Index        │
                                   │   (in-memory)     │    │    Manager        │
                                   └───────────────────┘    └─────────┬─────────┘
                                             ▲                        │
                                             │                        ▼
                             ┌─────────────┐         ┌───────────────────┐
                             │ Network     │<───┬────┤  Data Scratchpads  │
                             │ Adapter     │    │    │ (Autonomi Network)  │
                             └─────────────┘    │    └───────────────────┘
                                    ▲           │
                                    └───────────┘
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

- Defines `store`, `fetch`, `remove`, `update`.
- Default implementation (`DefaultDataManager`) delegates to `data::ops`:
  - `store_op` handles chunking, pad lifecycle, network writes, index updates.
  - `fetch_op` handles index lookup, pad fetches, reassembly.
  - `remove_op` handles index removal and pad recycling.

### 4.2 Indexing Layer (`IndexManager`)

- Maintains `MasterIndex` in-memory and persists via the network layer.
- Provides query and persistence interfaces.
- Ensures index consistency and supports remote index creation prompts.

### 4.3 Network Layer (`NetworkAdapter`)

- Abstract interface for network operations, including pad persistence:
  - Scratchpad reads/writes, existence checks.
- Default Autonomi adapter (`AutonomiNetworkAdapter`) uses `autonomi::Client`.
- Handles address/key derivation, serialization, encryption/decryption, and error translation.

### 4.4 Pad Lifecycle Layer (`PadLifecycleManager`)

- Manages free pad pool, allocation, preparation, and verification:
  - **Pool**: Tracks reusable pads harvested via `remove`.
  - **Prepare**: Generates new scratchpads when free pool is insufficient.
  - **Cache**: Persists index cache locally for performance.
  - **Import**: Imports external pads into free pool.
  - **Verification**: Confirms scratchpad writes before index update.

## 5. Core Data Structures

### 5.1 `MasterIndex`

```rust
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MasterIndex {
    pub(crate) index: Vec<(String, KeyInfo)>,
    pub(crate) free_pads: Vec<(ScratchpadAddress, Vec<u8>)>,
    pub(crate) scratchpad_size: usize,
}
```

- `index`: Maps each user key to its `KeyInfo`.
- `free_pads`: Recycled scratchpads with private keys for reuse.
- `scratchpad_size`: Configured usable byte size per pad.

### 5.2 `KeyInfo`

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyInfo {
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
2. Check `IndexManager` for existing key/resume info.
3. Ask `PadLifecycleManager` to acquire/prepare necessary pads (from free pool or generate new).
4. Chunk data; spawn concurrent write tasks via `NetworkAdapter`:
   - `put_raw` handles create/update logic based on pad status.
   - Report progress via `PutCallback`.
5. Update `IndexManager` with `KeyInfo` (pad addresses, keys, status).
6. Persist `IndexManager` state via `NetworkAdapter` (save master index).

### 6.2 Fetch

1. Query `IndexManager` for `KeyInfo`.
2. Spawn concurrent fetch tasks via `NetworkAdapter` (`get_raw_scratchpad`) to read & decrypt chunks using keys from `KeyInfo`.
3. Reassemble chunks in order; verify total size matches.
4. Return full `Vec<u8>`; report progress via `GetCallback`.

### 6.3 Remove

1. Remove key entry from `IndexManager`.
2. Harvest pads associated with the removed key using `IndexManager::harvest_pads`.
3. Persist updated `IndexManager` state via `NetworkAdapter`.

## 7. Error Model

Central `Error` enum unifies:

- Configuration errors
- Network, Index, PadLifecycle, Data errors (via `#[from]`)
- Callback cancellations & failures
- `NotImplemented`, `Internal` cases

## 8. Concurrency & Thread Safety

- The `MasterIndex` is wrapped in `Arc<Mutex<>>` within `IndexManager`, ensuring atomic updates.
- Data scratchpad operations run in parallel via `tokio::spawn` + `FuturesUnordered`.
- CPU-bound decryption handled within Autonomi SDK or potentially via `spawn_blocking` if needed elsewhere.

## 9. Encryption & Key Derivation

- Master index private key is derived from user key hex via SHA-256 → `SecretKey`.
- Pad private keys for new pads are randomly generated; for recycled pads loaded from `free_pads`.
- Data pad keys *are* stored encrypted within the `KeyInfo` in the `MasterIndex`.

## 10. Extensibility & Customization

- Trait-based abstractions primarily for `NetworkAdapter`.
- `DataManager`, `IndexManager`, `PadLifecycleManager` use concrete types (`Default...`).
- Users can implement custom network backends.
- `MutAntConfig` allows selecting network or tuning scratchpad size.

## 11. Further Reading

- `overview.md` for a concise introduction.
- `internals.md` for deep dives into algorithms and test coverage.
- Crate API docs on https://docs.rs/mutant_lib 