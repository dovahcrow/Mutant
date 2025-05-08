use anyhow::Result;

mod app;
mod callbacks;
mod cli;
mod commands;
mod history;
mod utils;

pub use app::connect_to_daemon;
use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(LevelFilter::Off)
        .parse_default_env()
        .init();

    app::run().await
}
