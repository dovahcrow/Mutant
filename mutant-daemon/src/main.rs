use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod app;
mod error;
mod handlers;
mod wallet;

use error::Error;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    alphanet: bool,
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
    };

    // Run the application
    app::run(options).await
}
