use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(short, long, default_value_t = false)]
    pub local: bool,

    #[arg(short, long, default_value_t = false)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    Put {
        key: String,
        value: Option<String>,
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },

    Get {
        key: String,
    },

    Rm {
        key: String,
    },

    Ls {
        #[arg(short, long, default_value_t = false)]
        long: bool,
    },

    Stats,

    Reset,

    Import {
        private_key: String,
    },

    #[command(about = "Synchronize local index cache with remote storage")]
    Sync {
        #[arg(long, default_value_t = false)]
        push_force: bool,
    },

    Purge,

    Reserve(crate::commands::reserve::Reserve),
}
