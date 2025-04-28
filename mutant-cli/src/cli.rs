use clap::Parser;
use clap::ValueEnum;
use mutant_protocol::StorageMode;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(clap::Subcommand)]
pub enum Commands {
    Put {
        key: String,
        file: String,
        #[arg(short, long)]
        public: bool,
        #[arg(value_enum, short, long, default_value_t = StorageModeCli::Medium)]
        mode: StorageModeCli,
        #[arg(short, long)]
        background: bool,
        #[arg(short, long)]
        no_verify: bool,
    },
    Get {
        key: String,
        destination_path: String,
        #[arg(short, long)]
        background: bool,
        #[arg(short, long)]
        public: bool,
    },
    Rm {
        key: String,
    },
    #[command(about = "List stored keys")]
    Ls,
    #[command(about = "Show storage statistics")]
    Stats,
    Tasks {
        #[command(subcommand)]
        command: TasksCommands,
    },
    Sync {
        #[arg(short, long)]
        push_force: bool,
        #[arg(short, long)]
        background: bool,
    },
    Purge {
        #[arg(short, long)]
        aggressive: bool,
        #[arg(short, long)]
        background: bool,
    },
    Import {
        file_path: String,
    },
    Export {
        destination_path: String,
    },
    HealthCheck {
        key_name: String,
        #[arg(short, long)]
        background: bool,
        #[arg(short, long)]
        recycle: bool,
    },
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

#[derive(clap::Subcommand)]
pub enum TasksCommands {
    List,
    Get { task_id: String },
}
