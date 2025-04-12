use clap::{Parser, Subcommand};
// use std::path::PathBuf;

/// Mutant CLI - Interact with the Mutant network
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to the wallet file (JSON containing private key string)
    // #[arg(short, long, value_name = "FILE", default_value = "mutant_wallet.json")]
    // pub wallet_path: PathBuf,

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
}
