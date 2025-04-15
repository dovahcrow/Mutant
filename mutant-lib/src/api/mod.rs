// Layer 5: Public API
// This module defines the public MutAnt struct and its methods, acting as the
// main entry point for the library. It orchestrates the underlying managers.

pub mod init;
pub mod mutant; // Keep MutAnt struct and impls in a separate file

pub use mutant::MutAnt; // Re-export the main struct
                        // Potentially re-export config/types if they aren't exposed at the top level lib.rs
                        // pub use crate::types::{MutAntConfig, KeyDetails, StorageStats};
                        // pub use crate::events::*; // Re-export events if needed by API users directly
