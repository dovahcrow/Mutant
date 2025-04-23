use clap::{Parser, Subcommand};

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
    Ls {
        #[arg(short, long, default_value_t = false)]
        long: bool,
    },

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
    // #[command(about = "Show storage statistics")]
    // Stats,

    // #[command(about = "Reset local cache and index")]
    // Reset,

    // #[command(about = "Import a scratchpad private key")]
    // Import { private_key: String },

    // #[command(about = "Synchronize local index cache with remote storage")]
    // Sync {
    //     #[arg(long, default_value_t = false)]
    //     push_force: bool,
    // },

    // #[command(
    //     about = "Perform a check on scratchpads that should have been created but maybe not and clean them up"
    // )]
    // Purge,

    // #[command(about = "Reserve a key without storing a value")]
    // Reserve(crate::commands::reserve::Reserve),
}
