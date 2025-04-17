//! MutAnt: Distributed storage SDK and CLI for Rust
//!
//! MutAnt provides a robust distributed storage system on top of the Autonomi network.
//!
//! Features:
//! - Chunked storage with configurable pad size
//! - Automatic pad reservation, write confirmation, and error retries
//! - Local and remote index caching for fast lookups
//! - CLI and library APIs for seamless integration
//!
//! Modules:
//! - `api`: High-level API for storage operations (init, put, get, remove, purge)
//! - `data`: Low-level data operations and chunk management
//! - `index`: Master index management and persistence
//! - `network`: Network adapter for scratchpad create/update/get
//! - `pad_lifecycle`: Pad acquisition, release, and verification workflow
//! - `storage`: Storage backend interfaces
//!
//! # Getting Started
//! Add to your `Cargo.toml`:
//! ```toml
//! mutant-lib = { path = "../mutant-lib" }
//! ```
//!
//! Basic usage:
//! ```rust
//! use mutant_lib::MutAnt;
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let private_key = "0x...".to_string();
//!     let mutant = MutAnt::init(private_key).await?;
//!     mutant.store("mykey".to_string(), b"hello").await?;
//!     let data = mutant.fetch("mykey").await?;
//!     println!("Data: {:?}", data);
//!     Ok(())
//! }
//! ```
//!
//! See the `mutant-cli` crate for command-line usage.
//! For details, refer to the documentation of individual modules.
//!
//! Contributing and license information can be found in the repository root.

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
