# MutAnt Library: Comprehensive Architecture & Implementation

This document provides an in-depth guide to the design, core data structures, component interactions, and operation flows within `mutant-lib`. It expands on both the high-level overview and the internal details to serve as a complete reference for users and contributors.

## 1. High-Level Overview

`mutant-lib` addresses the challenge of storing arbitrarily sized data blobs on the Autonomi network, which offers fixed-size scratchpads. It presents a cohesive key-value store abstraction with the following key concepts:

- **User Keys**: Human-friendly string identifiers for data blobs (e.g., `"config_v1"`)
- **Master Index**: A central data structure mapping user keys to metadata and tracking free pads
- **Data Scratchpads**: Regular scratchpads used to store encrypted chunks of user data
- **Worker Pool**: Manages concurrent operations with work distribution and recycling
- **Network Layer**: Low-level interface to the Autonomi network client, handling serialization, encryption, and network operations

## 2. Architecture Diagram

```
                                 ┌───────────────────────────────────────────┐
                                 │               MutAnt API                   │
                                 └───────────────────────────────────────────┘
                                                    │
                                                    ▼
                 ┌───────────────────────────────────────────────────────────────┐
                 │                             Data                               │
                 └───────────────────────────────────────────────────────────────┘
                    │                │                 │                  │
                    ▼                ▼                 ▼                  ▼
        ┌─────────────────┐  ┌─────────────┐  ┌─────────────────┐  ┌────────────┐
        │  Put Operation  │  │Get Operation│  │ Purge Operation │  │Other Ops   │
        └─────────────────┘  └─────────────┘  └─────────────────┘  └────────────┘
                    │                │                 │                  │
                    ▼                ▼                 ▼                  ▼
        ┌─────────────────────────────────────────────────────────────────────────┐
        │                           Worker Pool                                    │
        │  ┌─────────┐ ┌─────────┐ ┌─────────┐      ┌─────────┐                   │
        │  │ Worker 1 │ │ Worker 2│ │ Worker 3│ ... │ Worker N│                   │
        │  └─────────┘ └─────────┘ └─────────┘      └─────────┘                   │
        │        │           │           │                │                        │
        │        ▼           ▼           ▼                ▼                        │
        │  ┌─────────┐ ┌─────────┐ ┌─────────┐      ┌─────────┐                   │
        │  │ Client 1 │ │ Client 2│ │ Client 3│ ... │ Client N│                   │
        │  └─────────┘ └─────────┘ └─────────┘      └─────────┘                   │
        └─────────────────────────────────────────────────────────────────────────┘
                                        │
                                        ▼
                            ┌───────────────────────┐
                            │      Network          │
                            └───────────────────────┘
                                        │
                                        ▼
                            ┌───────────────────────┐
                            │  Autonomi Network     │
                            └───────────────────────┘
```

## 3. Public API (`MutAnt`)

All operations are async and return `Result<_, Error>`:

- **Initialization**
  - `init(private_key_hex)` - Initialize with mainnet
  - `init_public()` - Initialize for public fetching only
  - `init_local()` - Initialize with devnet
  - `init_alphanet(private_key_hex)` - Initialize with alphanet

- **Data Operations**
  - `put(key: &str, data: Arc<Vec<u8>>, mode: StorageMode, public: bool, no_verify: bool, callback: Option<PutCallback>)` - Store data
  - `get(key: &str, callback: Option<GetCallback>)` - Fetch data
  - `get_public(address: &ScratchpadAddress, callback: Option<GetCallback>)` - Fetch public data
  - `rm(key: &str)` - Remove data

- **Inspection & Maintenance**
  - `list() -> BTreeMap<String, IndexEntry>` - List all keys
  - `contains_key(key: &str) -> bool` - Check if key exists
  - `get_public_index_address(key: &str) -> String` - Get public address for a key
  - `get_storage_stats() -> StorageStats` - Get storage statistics
  - `purge(aggressive: bool, callback: Option<PurgeCallback>)` - Purge invalid pads
  - `health_check(key: &str, recycle: bool, callback: Option<HealthCheckCallback>)` - Check health of a key
  - `sync(force: bool, callback: Option<SyncCallback>)` - Sync with remote index

- **Import/Export**
  - `export_raw_pads_private_key() -> Vec<PadInfo>` - Export pad private keys
  - `import_raw_pads_private_key(pads: Vec<PadInfo>)` - Import pad private keys

## 4. Layer Breakdown

### 4.1 Data Layer (`Data`)

- Central coordinator for all data operations
- Delegates to specialized operation modules:
  - `put` module handles storing data, chunking, pad lifecycle, network writes, index updates
  - `get` module handles index lookup, pad fetches, reassembly
  - `purge` module handles verification and cleanup of invalid pads
  - `health_check` module verifies the integrity of stored data
  - `sync` module synchronizes the local index with the remote index

### 4.2 Indexing Layer (`MasterIndex`)

- Maintains the mapping between user keys and pad information
- Tracks free pads for reuse
- Handles public/private key management
- Provides methods for key operations (create, remove, update)
- Manages pad status tracking

### 4.3 Network Layer (`Network`)

- Provides an interface to the Autonomi network
- Manages client creation and configuration
- Handles scratchpad operations:
  - Reading and writing scratchpads
  - Encryption and decryption
  - Error handling and retries

### 4.4 Worker Pool (`WorkerPool`)

- Manages concurrent operations with configurable parallelism
- Distributes tasks to workers in round-robin fashion
- Implements work-stealing for efficient resource utilization
- Handles recycling of failed operations
- Monitors progress and completion of tasks

## 5. Core Data Structures

### 5.1 `MasterIndex`

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MasterIndex {
    /// Mapping from key names to their detailed information
    index: BTreeMap<String, IndexEntry>,

    /// List of scratchpads that are currently free and available for allocation
    free_pads: Vec<PadInfo>,

    /// List of scratchpads that are awaiting verification
    pending_verification_pads: Vec<PadInfo>,

    network_choice: NetworkChoice,
}
```

### 5.2 `IndexEntry`

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum IndexEntry {
    PrivateKey(Vec<PadInfo>),
    PublicUpload(PadInfo, Vec<PadInfo>),
}
```

### 5.3 `PadInfo`

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    pub address: ScratchpadAddress,
    pub private_key: Vec<u8>,
    pub status: PadStatus,
    pub size: usize,
    pub checksum: u64,
    pub last_known_counter: u64,
}
```

### 5.4 `PadStatus`

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum PadStatus {
    Generated,
    Written,
    Confirmed,
    Free,
    Invalid,
}
```

### 5.5 `WorkerPool`

```rust
pub struct WorkerPool<Item, Context, Client, Task, T, E> {
    pub(crate) task: Arc<Task>,
    pub(crate) clients: Vec<Arc<Client>>,
    pub(crate) worker_txs: Vec<Sender<Item>>,
    pub(crate) worker_rxs: Vec<Receiver<Item>>,
    pub(crate) global_tx: Sender<Item>,
    pub(crate) global_rx: Receiver<Item>,
    pub(crate) retry_sender: Option<Sender<(E, Item)>>,
    pub(crate) retry_rx: Option<Receiver<(E, Item)>>,
    pub(crate) total_items_hint: usize,
    // ... other fields
}
```

## 6. Operation Flows

### 6.1 Put Operation

1. Check if key exists:
   - If it exists with same data, resume the operation
   - If it exists with different data, update the key
   - If it doesn't exist, create a new key
2. For new keys:
   - Calculate chunk count based on data size and storage mode
   - Create pad entries in the index
   - Prepare worker pool with the appropriate task
3. For updates:
   - For public keys, preserve the public index pad
   - Remove the existing key
   - Create a new key with the updated data
   - For public keys, update the index pad
4. For all operations:
   - Distribute pads to workers in round-robin fashion
   - Workers process pads concurrently
   - Failed operations are recycled and retried
   - Update pad status as operations complete
   - Report progress via callbacks

### 6.2 Get Operation

1. Query the index for the key's pad information
2. Create a worker pool with the appropriate task
3. Distribute pads to workers
4. Workers fetch and decrypt pad data concurrently
5. Reassemble chunks in order
6. Return the complete data
7. Report progress via callbacks

### 6.3 Remove Operation

1. Remove the key entry from the index
2. Move associated pads to the free pad pool
3. Update the index

## 7. Worker Architecture

- Each worker manages a batch of concurrent tasks (default: 10)
- Workers are assigned a dedicated client to prevent blocking
- Tasks are distributed in round-robin fashion initially
- Workers can steal work from the global queue when their local queue is empty
- Failed operations are sent to a recycling queue
- A dedicated recycler task processes failed operations and redistributes them
- Completion is determined by counting confirmed pads

## 8. Concurrency & Thread Safety

- The `MasterIndex` is wrapped in `Arc<RwLock<>>`, allowing concurrent reads with exclusive writes
- The `Data` struct is wrapped in `Arc<RwLock<>>` for thread safety
- Worker pools use channels for communication between components
- Tasks are processed concurrently using tokio's async runtime
- Recycling mechanism ensures failed operations are retried

## 9. Public/Private Key Management

- Private keys are stored with encryption in the index
- Public keys have a special index pad that contains metadata about the data pads
- The public index pad address is used as the public address for the key
- When updating public keys, the index pad is preserved to maintain the same public address

## 10. Further Reading

- `overview.md` for a concise introduction
- `core_concepts.md` for fundamental concepts
- `internals/` directory for deep dives into specific components
- API docs on https://docs.rs/mutant_lib