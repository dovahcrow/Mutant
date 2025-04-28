use anyhow::Result;

mod app;
mod callbacks;
mod cli;
mod commands;
mod history;

pub use app::connect_to_daemon;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    app::run().await
}
