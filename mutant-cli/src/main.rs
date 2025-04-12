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
            if exit_code == ExitCode::SUCCESS {
                info!("MutAnt CLI finished successfully.");
            } else {
                error!("MutAnt CLI exited with error: {:?}", exit_code);
            }
            exit_code
        }
        Err(e) => {
            error!("MutAnt CLI exited with error: {}", e);
            ExitCode::FAILURE
        }
    }
}
