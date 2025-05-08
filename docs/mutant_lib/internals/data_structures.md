# Internals: Data Structures

This document details the key data structures used in MutAnt.

## 1. Master Index

The `MasterIndex` is the central data structure that maps user keys to pad information and tracks free pads.

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

### 1.1 Key Methods

- **create_key**: Creates a new key entry with the specified pads
- **remove_key**: Removes a key and moves its pads to the free_pads list
- **get_pads**: Gets all pads for a specific key
- **get_all_pads**: Gets all pads in the index
- **contains_key**: Checks if a key exists in the index
- **get_storage_stats**: Gets statistics about pad usage

## 2. Index Entry

The `IndexEntry` enum represents different types of key entries in the Master Index.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum IndexEntry {
    PrivateKey(Vec<PadInfo>),
    PublicUpload(PadInfo, Vec<PadInfo>),
}
```

- **PrivateKey**: A private key entry with a list of data pads
- **PublicUpload**: A public key entry with an index pad and a list of data pads

### 2.1 Usage

```rust
// Creating a private key entry
let private_entry = IndexEntry::PrivateKey(data_pads);

// Creating a public key entry
let public_entry = IndexEntry::PublicUpload(index_pad, data_pads);

// Accessing pads in an entry
match entry {
    IndexEntry::PrivateKey(pads) => {
        // Process private key pads
    },
    IndexEntry::PublicUpload(index_pad, data_pads) => {
        // Process public key index pad and data pads
    },
}
```

## 3. Pad Info

The `PadInfo` struct contains information about a single pad.

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

- **address**: The Autonomi scratchpad address
- **private_key**: The encrypted private key for the pad
- **status**: The current status of the pad
- **size**: The size of the data chunk stored in the pad
- **checksum**: A checksum of the data for verification
- **last_known_counter**: The last known counter value for the pad

## 4. Pad Status

The `PadStatus` enum represents the current state of a pad.

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

- **Generated**: Pad is allocated but not yet written to
- **Written**: Pad has been written to but not yet confirmed
- **Confirmed**: Pad has been successfully written and verified
- **Free**: Pad is available for reuse
- **Invalid**: Pad is invalid and cannot be used

## 5. Storage Mode

The `StorageMode` enum defines the chunk size used for storing data.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StorageMode {
    Small,   // 128KB chunks
    Medium,  // 2MB chunks
    Large,   // 8MB chunks
}
```

### 5.1 Chunk Size Calculation

```rust
impl StorageMode {
    pub fn chunk_size(&self) -> usize {
        match self {
            StorageMode::Small => 128 * 1024,    // 128KB
            StorageMode::Medium => 2 * 1024 * 1024,  // 2MB
            StorageMode::Large => 8 * 1024 * 1024,   // 8MB
        }
    }
}
```

## 6. Network Choice

The `NetworkChoice` enum specifies which Autonomi network to connect to.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum NetworkChoice {
    Mainnet,
    Devnet,
    Alphanet,
}
```

## 7. Worker Pool

The `WorkerPool` struct manages concurrent operations.

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
    pub(crate) monitor_processed_items: Arc<AtomicUsize>,
    pub(crate) monitor_active_workers: Arc<AtomicUsize>,
    pub(crate) monitor_all_items_processed: Arc<Notify>,
    pub(crate) monitor_total_items_hint: usize,
    pub(crate) worker_handles: Vec<JoinHandle<Result<(), E>>>,
    pub(crate) recycler_handle: Option<JoinHandle<Result<(), E>>>,
    pub(crate) _phantom: PhantomData<T>,
}
```

### 7.1 Key Components

- **task**: The task to be performed by workers
- **clients**: The clients used by workers
- **worker_txs/worker_rxs**: Channels for sending items to specific workers
- **global_tx/global_rx**: Channels for the global queue
- **retry_sender/retry_rx**: Channels for the recycling queue
- **monitor_processed_items**: Counter for processed items
- **monitor_active_workers**: Counter for active workers
- **monitor_all_items_processed**: Notification for completion
- **worker_handles**: Handles for worker tasks
- **recycler_handle**: Handle for the recycler task

## 8. Worker

The `Worker` struct processes items from its local queue and the global queue.

```rust
pub struct Worker<Item, Context, Client, Task, T, E> {
    pub(crate) id: usize,
    pub(crate) client: Arc<Client>,
    pub(crate) task: Arc<Task>,
    pub(crate) local_queue: Receiver<Item>,
    pub(crate) global_queue: Receiver<Item>,
    pub(crate) retry_tx: Option<Sender<(E, Item)>>,
    pub(crate) context: Arc<Context>,
    pub(crate) monitor_processed_items: Arc<AtomicUsize>,
    pub(crate) monitor_active_workers: Arc<AtomicUsize>,
    pub(crate) monitor_all_items_processed: Arc<Notify>,
    pub(crate) monitor_total_items_hint: usize,
    pub(crate) _phantom: PhantomData<T>,
}
```

### 8.1 Key Methods

- **run**: Processes items from the local queue and global queue
- **process_item**: Processes a single item
- **process_local_queue**: Processes items from the local queue
- **process_global_queue**: Processes items from the global queue

## 9. Task

The `Task` trait defines the operations that can be performed by workers.

```rust
pub trait Task<Item, Context, Client, T, E> {
    async fn process(
        &self,
        client: &Client,
        item: &Item,
        context: &Context,
    ) -> Result<T, E>;
}
```

### 9.1 Task Implementations

- **PutTask**: Writes data to a pad
- **GetTask**: Reads data from a pad
- **PurgeTask**: Verifies and cleans up invalid pads
- **HealthCheckTask**: Checks the health of a specific key's pads
- **SyncTask**: Synchronizes the local index with the remote index

## 10. Context

The `Context` struct contains information needed for a specific operation.

```rust
pub struct Context<T> {
    pub(crate) index: Arc<RwLock<MasterIndex>>,
    pub(crate) network: Arc<Network>,
    pub(crate) name: String,
    pub(crate) data: T,
    pub(crate) chunk_ranges: Vec<Range<usize>>,
}
```

### 10.1 Context Types

- **PutContext**: Contains data for a put operation
- **GetContext**: Contains data for a get operation
- **PurgeContext**: Contains data for a purge operation
- **HealthCheckContext**: Contains data for a health check operation
- **SyncContext**: Contains data for a sync operation

## 11. Callback Types

MutAnt provides several callback types for progress reporting.

```rust
pub type PutCallback = Box<dyn Fn(PutEvent) + Send + Sync>;
pub type GetCallback = Box<dyn Fn(GetEvent) + Send + Sync>;
pub type PurgeCallback = Box<dyn Fn(PurgeEvent) + Send + Sync>;
pub type HealthCheckCallback = Box<dyn Fn(HealthCheckEvent) + Send + Sync>;
pub type SyncCallback = Box<dyn Fn(SyncEvent) + Send + Sync>;
```

### 11.1 Event Types

- **PutEvent**: Events for put operations
- **GetEvent**: Events for get operations
- **PurgeEvent**: Events for purge operations
- **HealthCheckEvent**: Events for health check operations
- **SyncEvent**: Events for sync operations