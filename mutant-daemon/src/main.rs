use clap::Parser;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tokio::task::AbortHandle;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod app;
mod error;
mod handlers;
mod wallet;

use mutant_protocol::{Task, TaskId};

// Define a struct to hold task state and its abort handle
#[derive(Debug)]
struct TaskEntry {
    task: Task,
    abort_handle: Option<AbortHandle>,
}

// Update TaskMap to store TaskEntry instead of just Task
type TaskMap = Arc<RwLock<HashMap<TaskId, TaskEntry>>>;

use error::Error;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    alphanet: bool,
    #[arg(long)]
    private_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // Parse command line arguments
    let args = Args::parse();

    // Convert to app options
    let options = app::AppOptions {
        local: args.local,
        alphanet: args.alphanet,
        private_key: args.private_key,
    };

    // Run the application
    app::run(options).await
}
