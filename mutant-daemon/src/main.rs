use clap::Parser;

mod app;
mod colony;
mod error;
mod handlers;
mod wallet;

use error::Error;
use log::LevelFilter;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    alphanet: bool,
    #[arg(long)]
    ignore_ctrl_c: bool,
    #[arg(long, default_value = "127.0.0.1:3030", help = "Address to bind the WebSocket server to")]
    bind: String,
    #[arg(long, default_value = "/tmp/mutant-daemon.lock", help = "Path to the daemon lock file")]
    lock_file: String,
    #[arg(long, help = "Generate a new random testnet key on each run instead of using the testnet master key")]
    random_testnet_key: bool,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize logging
    env_logger::builder()
        .filter_level(LevelFilter::Off)
        .parse_default_env()
        .init();

    // Parse command line arguments
    let args = Args::parse();

    // Convert to app options
    let options = app::AppOptions {
        local: args.local,
        alphanet: args.alphanet,
        ignore_ctrl_c: args.ignore_ctrl_c,
        bind_address: args.bind,
        lock_file_path: args.lock_file,
        random_testnet_key: args.random_testnet_key,
    };

    // Run the application
    app::run(options).await
}
