//! # mutant-lib
//!
//! A library for storing and retrieving data on the Autonomi network's scratchpad storage,
//! managing data chunking, indexing, and pad lifecycle.

// Declare layer modules
pub mod api;
pub mod data;
pub mod index;
pub mod network;
pub mod pad_lifecycle;
pub mod storage;

// Declare top-level utility modules
pub mod error;
pub mod events;
pub mod types;

// --- Public Re-exports ---

// Core library entry point
pub use crate::api::MutAnt;

// Top-level error enum
pub use crate::error::Error;

// Public configuration and data structures
pub use crate::network::NetworkChoice;
pub use crate::types::{KeyDetails, MutAntConfig, StorageStats};

// Public events and callbacks
pub use crate::events::{
    GetCallback, GetEvent, InitCallback, InitProgressEvent, PurgeCallback, PurgeEvent, PutCallback,
    PutEvent,
};

// Re-export key external crates for convenience if needed by users
// (e.g., for constructing addresses or keys directly, though ideally abstracted)
pub use autonomi;
pub use log; // Useful for users to integrate with their logging setup

// Potentially hide internal details if not needed publicly
// mod internal_utils;
