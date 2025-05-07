use clap::Parser;

mod app;
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
    };

    // Run the application
    app::run(options).await
}
