//! # MutAnt
//!
//! MutAnt is a decentralized P2P mutable key-value storage system built on the Autonomi network, offering resilient, cost-efficient, and async-first storage with chunking, encryption, resumable uploads, and pad recycling.
//!
//! ## Why MutAnt?
//! Addressing on-chain storage limitations, MutAnt:
//! - **Splits** large data into scratchpad-sized chunks.
//! - **Resumes** interrupted transfers automatically.
//! - **Recycles** freed pads to reduce costs.
//! - **Caches** index locally for fast lookups and syncs remotely.
//! - **Encrypts** private data for secure storage.
//! - **Processes** operations in the background with task management.
//! - **Adapts** to business logic with pluggable backends.
//!
//! ## Key Highlights
//!
//! - **Chunk Management**: Configurable pad sizes with automatic chunking and reassembly.
//! - **Resumption & Retries**: Transparent retry logic and transfer continuation.
//! - **Cost Efficiency**: Reuses freed pads to minimize redundant on-chain writes.
//! - **Daemon Architecture**: Persistent daemon process handles network connections and operations.
//! - **Background Processing**: Run operations in the background with task management.
//! - **Public/Private Storage**: Store data publicly to share with others or privately with encryption.
//! - **Health Checks**: Verify and repair stored data with automatic pad recycling.
//! - **Flexible Interfaces**: Rust SDK (`mutant-lib`), WebSocket client (`mutant-client`), and CLI tool (`mutant`).
//! - **Async-First**: Built on `tokio` and `async/await`.
//! - **Extensible Architecture**: Modular design allows custom network layers.
//!
//! ## Ecosystem Components
//!
//! MutAnt consists of several components that work together:
//!
//! - **mutant-lib**: Core library handling chunking, encryption, and storage operations
//! - **mutant-protocol**: Shared communication format definitions
//! - **mutant-daemon**: Background service maintaining Autonomi connection
//! - **mutant-client**: WebSocket client library for communicating with the daemon
//! - **mutant-cli**: Command-line interface for end users
//!
//! ## Quickstart
//!
//! Add to `Cargo.toml`:
//! ```toml
//! mutant-lib = "0.6.0"
//! ```
//!
//! ```rust,no_run
//! use mutant_lib::MutAnt;
//! use mutant_lib::storage::StorageMode;
//! use anyhow::Result;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Use a dummy private key for doctest purposes.
//!     let key_hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
//!
//!     let mut ant = MutAnt::init(key_hex).await?;
//!
//!     // Store data with medium storage mode (2MB chunks)
//!     ant.put("file1", b"hello", StorageMode::Medium, false).await?;
//!
//!     // Retrieve the stored data
//!     let data = ant.get("file1").await?;
//!
//!     println!("Fetched: {}", String::from_utf8_lossy(&data));
//!     Ok(())
//! }
//! ```
//!
//! ### Fetching Public Data (without a private key)
//!
//! If you only need to fetch data that was stored publicly (using `put` with the public flag), you can
//! initialize a lightweight `MutAnt` instance without providing a private key:
//!
//! ```rust,no_run
//! use mutant_lib::MutAnt;
//! use mutant_lib::storage::ScratchpadAddress;
//! use anyhow::Result;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Initialize for public fetching (defaults to Mainnet)
//!     let public_fetcher = MutAnt::init_public().await?;
//!
//!     // You need the public address of the data (obtained elsewhere)
//!     let public_address = ScratchpadAddress::from_hex("...")?;
//!
//!     // Fetch the public data
//!     let data = public_fetcher.get_public(&public_address).await?;
//!
//!     println!("Fetched public data: {} bytes", data.len());
//!     Ok(())
//! }
//! ```
//!
//! **Note:** An instance created with `init_public` can *only* be used for `get_public`.
//! Other operations requiring a private key (like `put`, `get`, `remove`, etc.)
//! will fail.
//!
//! ### Using the Daemon and Client
//!
//! For most applications, it's recommended to use the daemon architecture:
//!
//! ```rust,no_run
//! use mutant_client::MutantClient;
//! use anyhow::Result;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Connect to the daemon (must be running)
//!     let mut client = MutantClient::new();
//!     client.connect("ws://localhost:3030/ws").await?;
//!
//!     // Start a put operation in the background
//!     let (start_task, progress_rx) = client.put(
//!         "my_key",
//!         "path/to/file.txt",
//!         mutant_protocol::StorageMode::Medium,
//!         false, // not public
//!         false, // verify
//!     ).await?;
//!
//!     // Monitor progress (optional)
//!     tokio::spawn(async move {
//!         while let Ok(progress) = progress_rx.recv().await {
//!             println!("Progress: {:?}", progress);
//!         }
//!     });
//!
//!     // Wait for the task to complete
//!     let result = start_task.await?;
//!     println!("Task completed: {:?}", result);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Resources & Support
//!
//! - API docs   : https://docs.rs/mutant_lib
//! - CLI help   : `mutant --help`
//! - Repository : https://github.com/Champii/MutAnt
//! - Issues     : https://github.com/Champii/MutAnt/issues
//!

/// Provides the main API entry point for interacting with MutAnt.
mod api;
/// Manages indexing and search functionality for stored data.
mod index;
/// Contains network-related functionalities, including data persistence via scratchpads.
mod network;
/// Handles data structures and serialization/deserialization logic, including worker pools.
mod ops;

/// Defines custom error types used throughout the `mutant-lib`.
mod internal_error;
/// Defines events and callbacks used for asynchronous operations and progress reporting.
mod internal_events;

// Re-export main API entry point
pub use crate::api::MutAnt;

pub mod config {
    pub use crate::network::{NetworkChoice, DEV_TESTNET_PRIVATE_KEY_HEX};
}

pub mod storage {
    pub use super::network::{GetResult, PutResult};
    pub use crate::index::master_index::IndexEntry;
    pub use crate::index::pad_info::{PadInfo, PadStatus};
    pub use autonomi::ScratchpadAddress;
    pub use mutant_protocol::StorageMode;
}

pub mod error {
    pub use crate::internal_error::Error;
    pub use crate::ops::worker::PoolError;
}

pub mod events {
    pub use mutant_protocol::{
        GetCallback, GetEvent, HealthCheckCallback, HealthCheckEvent, InitCallback,
        InitProgressEvent, PurgeCallback, PurgeEvent, PutCallback, PutEvent,
        SyncCallback, SyncEvent, TaskProgress, TaskResult, TaskStatus, TaskType,
    };
}

pub mod worker {
    pub use crate::ops::worker::{AsyncTask, WorkerPoolConfig};
}
