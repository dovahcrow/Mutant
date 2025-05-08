# Internals: Concurrency and Thread Safety

`mutant-lib` is designed to be used in asynchronous Rust applications and needs to handle concurrent operations safely and efficiently. This document details the concurrency model used throughout the library.

## 1. Shared State Protection

### 1.1 Master Index Protection

The most critical shared resource is the `MasterIndex`, which maps user keys to pad information and tracks free pads. Multiple operations might need to read or modify this data structure concurrently.

To ensure thread safety and prevent data races, the `MasterIndex` is wrapped in `Arc<RwLock<>>`:

```rust
// Within the MutAnt struct
index: Arc<RwLock<MasterIndex>>,
```

- **`Arc` (Atomically Reference Counted):** Allows multiple owners of the data across different tasks and threads.
- **`RwLock` (Read-Write Lock):** Provides more granular locking than a `Mutex`:
  - Multiple readers can access the data simultaneously
  - Writers get exclusive access, blocking all other readers and writers
  - This is more efficient than a `Mutex` for read-heavy workloads

**Usage Pattern:**

```rust
// For read-only operations
let index_guard = self.index.read().await;
let result = index_guard.some_read_operation();
drop(index_guard);  // Explicitly release the lock

// For write operations
let mut index_guard = self.index.write().await;
index_guard.some_write_operation()?;
drop(index_guard);  // Explicitly release the lock
```

### 1.2 Data Layer Protection

The `Data` struct, which coordinates operations, is also protected with an `Arc<RwLock<>>`:

```rust
// Within the MutAnt struct
data: Arc<RwLock<Data>>,
```

This allows multiple concurrent read operations (like `get` or `list`) while ensuring that write operations (like `put`) have exclusive access when needed.

## 2. Worker Pool Architecture

The worker pool is the heart of MutAnt's concurrency model, designed to efficiently process operations in parallel.

### 2.1 Pool Structure

```
┌─────────────────────────────────────────────────────────────────┐
│                        Worker Pool                               │
│                                                                  │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐        ┌─────────┐     │
│  │ Worker 1 │   │ Worker 2 │   │ Worker 3 │  ...  │ Worker N │     │
│  └─────────┘   └─────────┘   └─────────┘        └─────────┘     │
│       │             │             │                  │           │
│       ▼             ▼             ▼                  ▼           │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐        ┌─────────┐     │
│  │ Client 1 │   │ Client 2 │   │ Client 3 │  ...  │ Client N │     │
│  └─────────┘   └─────────┘   └─────────┘        └─────────┘     │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                    Global Queue                          │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                    Recycling Queue                       │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 Key Components

1. **Workers**: Each worker manages a batch of concurrent tasks (default: 10 tasks per worker)
2. **Clients**: Each worker has exactly ONE dedicated client to prevent blocking
3. **Local Queues**: Each worker has a local queue for assigned tasks
4. **Global Queue**: A shared queue that all workers can pull from when their local queue is empty
5. **Recycling Queue**: A queue for failed operations that need to be retried

### 2.3 Channel-Based Communication

The worker pool uses `async_channel` for communication between components:

```rust
// Worker pool channels
worker_txs: Vec<Sender<Item>>,      // Send items to specific workers
worker_rxs: Vec<Receiver<Item>>,    // Workers receive items from their channel
global_tx: Sender<Item>,            // Send items to the global queue
global_rx: Receiver<Item>,          // Workers pull from global queue when local is empty
retry_sender: Option<Sender<(E, Item)>>,  // Send failed items for recycling
retry_rx: Option<Receiver<(E, Item)>>,    // Recycler receives failed items
```

This channel-based approach provides a blocking mechanism that doesn't consume CPU while waiting for work.

## 3. Task Distribution and Work Stealing

### 3.1 Initial Distribution

Tasks are initially distributed to workers in a round-robin fashion:

```rust
let mut worker_index = 0;
for item in items {
    let target_tx = &self.worker_txs[worker_index % num_workers];
    target_tx.send(item).await?;
    worker_index += 1;
}
```

### 3.2 Work Stealing

Workers can "steal" work from the global queue when their local queue is empty:

```rust
// Simplified version of the worker's task processing loop
tokio::select! {
    biased;
    result = self.local_queue.recv() => {
        // Process item from local queue
    },
    result = self.global_queue.recv() => {
        // Process item from global queue (work stealing)
    },
}
```

The `biased` keyword ensures that the local queue is always checked first, prioritizing assigned work.

## 4. Recycling Mechanism

Failed operations are sent to a recycling queue for retry:

```rust
// When an operation fails
if let Some(retry_tx) = &self.retry_sender {
    retry_tx.try_send((error.clone(), failed_item))?;
}
```

A dedicated recycler task processes these failed items:

```rust
match recycle_function(error_cause, item_to_recycle).await {
    Ok(Some(new_item)) => {
        // Send recycled item back to the global queue
        global_tx_for_recycler.send(new_item).await?;
    },
    Ok(None) => {
        // Item cannot be recycled, drop it
    },
    Err(recycle_err) => {
        // Recycling failed, log error
    }
}
```

## 5. Completion Detection

The worker pool monitors completion using several mechanisms:

1. **Processed Items Counter**: Tracks how many items have been successfully processed
2. **Active Workers Counter**: Tracks how many worker tasks are still active
3. **Total Items Hint**: The expected number of items to process

Completion is determined when either:
- The processed items count matches the total items hint (all pads confirmed)
- All workers have completed and at least one item was processed

```rust
if processed_items > 0 && processed_items == monitor_total_items_hint {
    // All expected items processed
    monitor_all_items_processed.notify_waiters();
} else if processed_items > 0 && active_workers == 0 {
    // All workers done but fewer items than expected
    monitor_all_items_processed.notify_waiters();
}
```

## 6. Asynchronous Execution with Tokio

The library heavily relies on the `tokio` runtime for asynchronous operations:

- **`async`/`await`**: Used throughout the codebase for non-blocking I/O
- **`tokio::spawn`**: Used to launch concurrent tasks for workers and recyclers
- **`tokio::sync::RwLock`**: Used for protecting shared state with reader/writer semantics
- **`tokio::sync::Notify`**: Used for signaling completion between tasks
- **`tokio::select!`**: Used for efficiently waiting on multiple async operations

## Summary

Concurrency in `mutant-lib` is managed through:

1. An `Arc<RwLock<>>` protecting shared state like the `MasterIndex` and `Data`
2. A worker pool architecture with dedicated clients and channel-based communication
3. Round-robin task distribution with work stealing for efficient resource utilization
4. A recycling mechanism for handling failed operations
5. Sophisticated completion detection based on processed items and active workers

This architecture provides both thread safety and high throughput for network-bound operations, with efficient CPU utilization and resilience against transient failures.