mod app;
mod callbacks;
mod cli;
mod commands;

use env_logger::{Builder, Env};
use log::{error, info};
use std::process::ExitCode;

use crate::app::run_cli;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = Builder::from_env(Env::default().default_filter_or("info")).try_init() {
        eprintln!("Failed to initialize logger: {}", e);
        return ExitCode::FAILURE;
    }

    match run_cli().await {
        Ok(exit_code) => {
            info!("Anthill CLI finished successfully.");
            exit_code
        }
        Err(e) => {
            error!("Anthill CLI exited with error: {}", e);
            ExitCode::FAILURE
        }
    }
}
