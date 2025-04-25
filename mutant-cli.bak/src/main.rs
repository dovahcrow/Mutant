mod app;
mod callbacks;
mod cli;
mod commands;
mod history;

use env_logger::{Builder, Env};
use log::{debug, error, info};
use std::process::ExitCode;

use crate::app::run_cli;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = Builder::from_env(Env::default().default_filter_or("error")).try_init() {
        eprintln!("Failed to initialize logger: {}", e);
        return ExitCode::FAILURE;
    }

    debug!("Entering main, calling run_cli...");
    match run_cli().await {
        Ok(exit_code) => {
            debug!("run_cli returned successfully with code: {:?}", exit_code);
            if exit_code == ExitCode::SUCCESS {
                info!("MutAnt CLI finished successfully.");
            } else {
                error!("MutAnt CLI exited with error: {:?}", exit_code);
            }
            exit_code
        }
        Err(e) => {
            debug!("run_cli returned error: {}", e);
            error!("MutAnt CLI Error: {}", e);
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}
