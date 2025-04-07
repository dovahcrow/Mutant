use anthill_lib::autonomi::{Network, Wallet};
use anthill_lib::{anthill::Anthill, events::InitCallback};
use clap::Parser;
use indicatif::MultiProgress;
use log::{debug, error, info};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::task::JoinHandle;

use crate::callbacks::{StyledProgressBar, create_init_callback};
use crate::cli::Cli;
use crate::commands::handle_command;

#[derive(Debug)]
pub enum CliError {
    WalletRead(std::io::Error, std::path::PathBuf),
    WalletParse(serde_json::Error, std::path::PathBuf),
    NetworkInit(String),
    WalletCreate(String),
    AnthillInit(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::WalletRead(e, path) => {
                write!(f, "Error reading private key from {:?}: {}", path, e)
            }
            CliError::WalletParse(e, path) => {
                write!(f, "Error parsing JSON from wallet file {:?}: {}", path, e)
            }
            CliError::NetworkInit(e) => write!(f, "Error initializing network: {}", e),
            CliError::WalletCreate(e) => write!(f, "Error creating wallet: {}", e),
            CliError::AnthillInit(e) => write!(f, "Error during Anthill initialization: {}", e),
        }
    }
}

impl std::error::Error for CliError {}

async fn initialize_wallet(wallet_path: &PathBuf) -> Result<(Wallet, String), CliError> {
    let private_key_hex = {
        let content = fs::read_to_string(wallet_path)
            .map_err(|e| CliError::WalletRead(e, wallet_path.clone()))?;
        serde_json::from_str::<String>(&content)
            .map_err(|e| CliError::WalletParse(e, wallet_path.clone()))?
    };
    debug!("Read private key hex from file: '{}'", private_key_hex);

    let network = Network::new(true).map_err(|e| CliError::NetworkInit(e.to_string()))?;

    let wallet = match Wallet::new_from_private_key(network.clone(), &private_key_hex) {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to initialize wallet: {}", e);
            return Err(CliError::WalletCreate(e.to_string()));
        }
    };
    info!("Wallet created using key from {:?}", wallet_path);

    Ok((wallet, private_key_hex))
}

async fn cleanup_background_tasks(
    mp_join_handle: JoinHandle<()>,
    anthill_init_handle: Option<JoinHandle<()>>,
) {
    if !mp_join_handle.is_finished() {
        debug!("Aborting and awaiting MultiProgress drawing task...");
        mp_join_handle.abort();
        if let Err(e) = mp_join_handle.await {
            if !e.is_cancelled() {
                error!("MultiProgress join handle error after abort: {}", e);
            }
        }
        debug!("MultiProgress drawing task finished.");
    }

    if let Some(handle) = anthill_init_handle {
        info!("Waiting for background Anthill/Storage task to complete...");
        match handle.await {
            Ok(_) => {
                info!("Background Anthill/Storage task finished successfully.");
            }
            Err(e) => {
                if e.is_panic() {
                    error!("Background Anthill/Storage task panicked: {}", e);
                } else if e.is_cancelled() {
                    info!("Background Anthill/Storage task was cancelled.");
                } else {
                    error!("Background Anthill/Storage task failed to join: {}", e);
                }
            }
        }
    }
}

pub async fn run_cli() -> Result<ExitCode, CliError> {
    info!("Anthill CLI started processing.");

    let cli_args = Cli::parse();

    let (wallet, private_key_hex) = initialize_wallet(&cli_args.wallet_path).await?;

    let multi_progress = MultiProgress::new();
    let (init_pb, init_callback): (StyledProgressBar, InitCallback) =
        create_init_callback(&multi_progress);

    let mp_join_handle = tokio::spawn(async move {
        std::future::pending::<()>().await;
    });

    info!("Initializing Anthill layer (including Storage)...");
    let (anthill, anthill_init_handle) =
        match Anthill::init(wallet, private_key_hex, Some(init_callback)).await {
            Ok((a, h)) => (a, h),
            Err(e) => {
                if !init_pb.is_finished() {
                    init_pb.abandon_with_message("Initialization Failed".to_string());
                }
                mp_join_handle.abort();
                let _ = mp_join_handle.await;
                return Err(CliError::AnthillInit(e.to_string()));
            }
        };

    if !init_pb.is_finished() {
        init_pb.finish_and_clear();
    }

    let exit_code = handle_command(cli_args.command, anthill, &multi_progress).await;

    info!("Anthill CLI command finished processing.");

    cleanup_background_tasks(mp_join_handle, anthill_init_handle).await;

    Ok(exit_code)
}
