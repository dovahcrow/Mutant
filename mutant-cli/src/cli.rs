use clap::{Parser, Subcommand, ValueEnum};
use mutant_lib::storage::StorageMode;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(
        short,
        long,
        default_value_t = false,
        help = "Use local network (Devnet)"
    )]
    pub local: bool,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "Suppress progress bar output"
    )]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum StorageModeCli {
    /// 0.5 MB per scratchpad
    Lightest,
    /// 1 MB per scratchpad
    Light,
    /// 2 MB per scratchpad
    Medium,
    /// 3 MB per scratchpad
    Heavy,
    /// 4 MB per scratchpad
    Heaviest,
}

impl From<StorageModeCli> for StorageMode {
    fn from(mode: StorageModeCli) -> Self {
        match mode {
            StorageModeCli::Lightest => StorageMode::Lightest,
            StorageModeCli::Light => StorageMode::Light,
            StorageModeCli::Medium => StorageMode::Medium,
            StorageModeCli::Heavy => StorageMode::Heavy,
            StorageModeCli::Heaviest => StorageMode::Heaviest,
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    #[command(about = "Store a value associated with a key")]
    Put {
        key: String,
        value: Option<String>,
        #[arg(short, long, default_value_t = false)]
        force: bool,
        #[arg(short, long, default_value_t = false)]
        public: bool,
        #[arg(value_enum, short, long, default_value_t = StorageModeCli::Medium)]
        mode: StorageModeCli,
    },

    #[command(about = "Retrieve a value associated with a key")]
    Get {
        key: String,
        #[arg(short, long, default_value_t = false)]
        public: bool,
    },

    #[command(about = "Remove a key-value pair")]
    Rm { key: String },

    #[command(about = "List stored keys")]
    Ls,

    #[command(about = "Export all scratchpad private key to a file")]
    Export {
        #[arg(default_value = "pads.hex")]
        output: String,
    },

    #[command(about = "Import scratchpad private key from a file")]
    Import {
        #[arg(default_value = "pads.hex")]
        input: String,
    },
    #[command(about = "Show storage statistics")]
    Stats,

    // #[command(about = "Reset local cache and index")]
    // Reset,

    // #[command(about = "Synchronize local index cache with remote storage")]
    // Sync {
    //     #[arg(long, default_value_t = false)]
    //     push_force: bool,
    // },
    #[command(
        about = "Perform a get check on scratchpads that should have been created but failed at some point. Removes the pads that are not found."
    )]
    Purge {
        #[arg(
            short,
            long,
            default_value_t = false,
            help = "Remove also the pads that got any other error"
        )]
        aggressive: bool,
    },

    #[command(
        about = "Perform a health check on scratchpads that should have been created but cannot be retrieved. Recycles the pads that are not found."
    )]
    HealthCheck { key: String },
    // #[command(about = "Reserve a key without storing a value")]
    // Reserve(crate::commands::reserve::Reserve),
}
