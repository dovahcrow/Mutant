use clap::{Parser, Subcommand};
// use std::path::PathBuf;

/// Mutant CLI - Interact with the Mutant network
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Use local network (Devnet) instead of Mainnet
    #[arg(short, long, default_value_t = false)]
    pub local: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Puts a key-value pair onto the network. Reads value from stdin if omitted.
    /// Use --force to overwrite an existing key.
    Put {
        key: String,
        value: Option<String>,
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// Gets the value for a given key from the network and prints it to stdout.
    Get { key: String },
    /// Deletes a key-value pair from the network
    Rm { key: String },
    /// Lists all keys stored on the network
    Ls {
        #[arg(short, long, default_value_t = false)]
        long: bool,
    },
    /// Get storage summary (allocator perspective)
    Stats,
    /// Resets the master index to its initial empty state. Requires confirmation.
    Reset,
    /// Imports a free scratchpad using its private key.
    Import { private_key: String },

    /// Synchronize local index cache with remote storage
    #[command(about = "Synchronize local index cache with remote storage")]
    Sync {
        /// Overwrite remote index with local one
        #[arg(long, default_value_t = false)]
        push_force: bool,
    },
}
