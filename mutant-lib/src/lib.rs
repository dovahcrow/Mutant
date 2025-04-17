//! # MutAnt
//!
//! MutAnt is a private, mutable key-value store built on Autonomi network scratchpads, offering resilient, cost-efficient, and async-first storage.
//!
//! ## Why MutAnt?
//! Addressing on-chain storage limitations, MutAnt:
//! - **Splits** large data into scratchpad-sized chunks.
//! - **Resumes** interrupted transfers automatically.
//! - **Recycles** freed pads to reduce costs.
//! - **Caches** index locally for fast lookups and syncs remotely.
//! - **Adapts** to business logic with pluggable backends.
//!
//! ## Key Highlights
//!
//! - **Chunk Management**: Configurable pad sizes with automatic chunking and reassembly.
//! - **Resumption & Retries**: Transparent retry logic and transfer continuation.
//! - **Cost Efficiency**: Reuses freed pads to minimize redundant on-chain writes.
//! - **Flexible Interfaces**: Rust SDK (`mutant-lib`) and CLI tool (`mutant`).
//! - **Async-First**: Built on `tokio` and `async/await`.
//! - **Extensible Architecture**: Modular design allows custom network/storage layers.
//!
//! ## Quickstart
//!
//! Add to `Cargo.toml`:
//! ```toml
//! mutant-lib = { path = "../mutant-lib" }
//! ```
//!
//! ```rust
//! use mutant_lib::{MutAnt, MutAntConfig};
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let key_hex = "0xYOUR_PRIVATE_KEY_HEX".to_string();
//!     let mut ant = MutAnt::init_with_progress(key_hex, MutAntConfig::default(), None).await?;
//!     ant.store("file1".into(), b"hello").await?;
//!     let data = ant.fetch("file1").await?;
//!     println!("Fetched: {}", String::from_utf8_lossy(&data));
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
/// Handles data structures and serialization/deserialization logic.
mod data;
/// Manages indexing and search functionality for stored data.
mod index;
/// Contains network-related functionalities, including peer discovery and data synchronization.
mod network;
/// Manages the lifecycle of pads, including creation, deletion, and updates.
mod pad_lifecycle;
/// Defines storage backends and interfaces for persisting data.
mod storage;

/// Defines custom error types used throughout the `mutant-lib`.
mod error;
/// Defines events and callbacks used for asynchronous operations and progress reporting.
mod events;
/// Contains core data types and configuration structures used by MutAnt.
mod types;

/// Re-exports the main MutAnt API structure.
pub use crate::api::MutAnt;
/// Re-export API event types needed by CLI
pub use crate::api::{ReserveCallback, ReserveEvent};

/// Re-export submodule errors needed by CLI
pub use crate::data::error::DataError;
/// Re-exports the primary error type for the library.
pub use crate::error::Error;
pub use crate::index::error::IndexError;
pub use crate::pad_lifecycle::error::PadLifecycleError;

/// Re-exports the `NetworkChoice` enum for selecting network backends.
pub use crate::network::NetworkChoice;
/// Re-exports key data structures related to configuration, keys, and storage statistics.
pub use crate::types::{KeyDetails, MutAntConfig, StorageStats};

/// Re-exports various event and callback types related to core operations like get, put, init, and purge.
pub use crate::events::{
    GetCallback, GetEvent, InitCallback, InitProgressEvent, PurgeCallback, PurgeEvent, PutCallback,
    PutEvent,
};

/// Re-export dependency types needed by CLI
pub use autonomi::ScratchpadAddress;
