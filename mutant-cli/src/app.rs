use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use directories::{BaseDirs, ProjectDirs};
use indicatif::MultiProgress;
use log::{debug, error, info, warn};
use mutant_lib::autonomi::{Network, Wallet};
use mutant_lib::{
    events::InitCallback,
    mutant::{MutAnt, MutAntConfig, NetworkChoice},
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tokio::task::JoinHandle;

use crate::callbacks::{StyledProgressBar, create_init_callback};
use crate::cli::Cli;
use crate::commands::handle_command;

#[derive(Serialize, Deserialize, Debug, Default)]
struct MutantCliConfig {
    wallet_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum CliError {
    WalletRead(io::Error, PathBuf),
    NetworkInit(String),
    WalletCreate(String),
    MutAntInit(String),
    ConfigDirNotFound,
    ConfigRead(io::Error, PathBuf),
    ConfigParse(serde_json::Error, PathBuf),
    ConfigWrite(io::Error, PathBuf),
    WalletDirNotFound,
    WalletDirRead(io::Error, PathBuf),
    NoWalletsFound(PathBuf),
    UserSelectionFailed(dialoguer::Error),
    WalletNotSet,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::WalletRead(e, path) => {
                write!(f, "Error reading private key from {:?}: {}", path, e)
            }
            CliError::NetworkInit(e) => write!(f, "Error initializing network: {}", e),
            CliError::WalletCreate(e) => write!(f, "Error creating wallet: {}", e),
            CliError::MutAntInit(e) => write!(f, "Error during MutAnt initialization: {}", e),
            CliError::ConfigDirNotFound => write!(f, "Could not find configuration directory."),
            CliError::ConfigRead(e, path) => {
                write!(f, "Error reading config file {:?}: {}", path, e)
            }
            CliError::ConfigParse(e, path) => {
                write!(f, "Error parsing config file {:?}: {}", path, e)
            }
            CliError::ConfigWrite(e, path) => {
                write!(f, "Error writing config file {:?}: {}", path, e)
            }
            CliError::WalletDirNotFound => write!(f, "Could not find Autonomi wallet directory."),
            CliError::WalletDirRead(e, path) => {
                write!(f, "Error reading wallet directory {:?}: {}", path, e)
            }
            CliError::NoWalletsFound(path) => write!(f, "No wallet files found in {:?}", path),
            CliError::UserSelectionFailed(e) => {
                write!(f, "Failed to get user wallet selection: {}", e)
            }
            CliError::WalletNotSet => write!(f, "No wallet configured or selected."),
        }
    }
}

impl std::error::Error for CliError {}

fn get_config_path() -> Result<PathBuf, CliError> {
    let proj_dirs =
        ProjectDirs::from("com", "Mutant", "MutantCli").ok_or(CliError::ConfigDirNotFound)?;
    let config_dir = proj_dirs.config_dir();
    if !config_dir.exists() {
        fs::create_dir_all(config_dir)
            .map_err(|e| CliError::ConfigWrite(e, config_dir.to_path_buf()))?;
    }
    Ok(config_dir.join("mutant.json"))
}

fn load_config(config_path: &Path) -> Result<MutantCliConfig, CliError> {
    if !config_path.exists() {
        info!("Config file {:?} not found, using default.", config_path);
        return Ok(MutantCliConfig::default());
    }
    let content = fs::read_to_string(config_path)
        .map_err(|e| CliError::ConfigRead(e, config_path.to_path_buf()))?;
    serde_json::from_str(&content).map_err(|e| CliError::ConfigParse(e, config_path.to_path_buf()))
}

fn save_config(config_path: &Path, config: &MutantCliConfig) -> Result<(), CliError> {
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| CliError::ConfigParse(e, config_path.to_path_buf()))?; // Should not happen with our struct
    fs::write(config_path, content).map_err(|e| CliError::ConfigWrite(e, config_path.to_path_buf()))
}

fn get_autonomi_wallet_dir() -> Result<PathBuf, CliError> {
    let base_dirs = BaseDirs::new().ok_or(CliError::WalletDirNotFound)?;
    // XDG_DATA_HOME is preferred
    let data_dir = base_dirs.data_dir();
    let wallet_dir = data_dir.join("autonomi/client/wallets");
    if wallet_dir.is_dir() {
        Ok(wallet_dir)
    } else {
        // Fallback for non-XDG environments or different setups might be needed?
        // For now, we'll just report the specific path wasn't found.
        warn!(
            "Standard Autonomi wallet directory not found at {:?}",
            wallet_dir
        );
        Err(CliError::WalletDirNotFound)
    }
}

fn scan_wallet_dir(wallet_dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    let entries = fs::read_dir(wallet_dir)
        .map_err(|e| CliError::WalletDirRead(e, wallet_dir.to_path_buf()))?;
    let mut wallets = Vec::new();
    for entry_result in entries {
        let entry =
            entry_result.map_err(|e| CliError::WalletDirRead(e, wallet_dir.to_path_buf()))?;
        let path = entry.path();
        if path.is_file() {
            // Basic check: filename starts with 0x and has typical hex length?
            // This is a weak check, ideally Autonomi defines a standard.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("0x") && name.len() > 40 {
                    // Very basic heuristic
                    wallets.push(path);
                }
            }
        }
    }
    if wallets.is_empty() {
        Err(CliError::NoWalletsFound(wallet_dir.to_path_buf()))
    } else {
        Ok(wallets)
    }
}

fn prompt_user_for_wallet(wallets: &[PathBuf]) -> Result<PathBuf, CliError> {
    if wallets.is_empty() {
        // This case should be handled by the caller checking scan_wallet_dir result
        return Err(CliError::WalletNotSet);
    }
    if wallets.len() == 1 {
        info!("Only one wallet found, using it: {:?}", wallets[0]);
        return Ok(wallets[0].clone());
    }

    let items: Vec<String> = wallets
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    info!("Multiple wallets found. Please select one to use:");
    let selection = Select::with_theme(&ColorfulTheme::default())
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(CliError::UserSelectionFailed)?;

    match selection {
        Some(index) => Ok(wallets[index].clone()),
        None => {
            error!("No wallet selected by the user.");
            Err(CliError::WalletNotSet) // Or a more specific "UserCancelled" error
        }
    }
}

async fn initialize_wallet() -> Result<(Wallet, String), CliError> {
    let config_path = get_config_path()?;
    let mut config = load_config(&config_path)?;

    let wallet_path = loop {
        if let Some(ref path) = config.wallet_path {
            if path.exists() {
                info!("Using wallet from config: {:?}", path);
                break path.clone();
            } else {
                warn!(
                    "Wallet path from config {:?} does not exist. Rescanning.",
                    path
                );
                config.wallet_path = None; // Invalidate missing path
            }
        }

        // Wallet path not in config or invalid, try scanning
        info!("No valid wallet in config, scanning Autonomi wallet directory...");
        let wallet_dir = get_autonomi_wallet_dir()?;
        let available_wallets = scan_wallet_dir(&wallet_dir)?;

        let selected_wallet = prompt_user_for_wallet(&available_wallets)?;
        info!("Selected wallet: {:?}", selected_wallet);

        // Save the selection to config
        config.wallet_path = Some(selected_wallet.clone());
        save_config(&config_path, &config)?;
        info!("Saved selected wallet path to config: {:?}", config_path);
        break selected_wallet;
    };

    let private_key_hex = {
        let content = fs::read_to_string(&wallet_path)
            .map_err(|e| CliError::WalletRead(e, wallet_path.clone()))?;
        // Assuming the file *only* contains the private key string directly, not JSON
        debug!("Raw content read from wallet file: '{}'", content.trim());
        content.trim().to_string()
        // If the wallet file *is* JSON containing a string:
        // serde_json::from_str::<String>(&content)
        //     .map_err(|e| CliError::WalletParse(e, wallet_path.clone()))?
    };
    debug!("Using private key hex from file: '{}'", private_key_hex);

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
    mutant_init_handle: Option<JoinHandle<()>>,
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

    if let Some(handle) = mutant_init_handle {
        info!("Waiting for background MutAnt/Storage task to complete...");
        match handle.await {
            Ok(_) => {
                info!("Background MutAnt/Storage task finished successfully.");
            }
            Err(e) => {
                if e.is_panic() {
                    error!("Background MutAnt/Storage task panicked: {}", e);
                } else if e.is_cancelled() {
                    info!("Background MutAnt/Storage task was cancelled.");
                } else {
                    error!("Background MutAnt/Storage task failed to join: {}", e);
                }
            }
        }
    }
}

pub async fn run_cli() -> Result<ExitCode, CliError> {
    info!("MutAnt CLI started processing.");

    let cli_args = Cli::parse(); // Note: Cli struct needs update to remove wallet_path

    // Initialize wallet using the new config/scan/prompt logic
    let (_wallet, private_key_hex) = initialize_wallet().await?;

    let multi_progress = MultiProgress::new();
    let (init_pb, init_callback): (StyledProgressBar, InitCallback) =
        create_init_callback(&multi_progress);

    // This handle is just to keep the MultiProgress drawing alive until cleanup
    let mp_join_handle = tokio::spawn(async move {
        std::future::pending::<()>().await; // Keep the drawing task alive indefinitely
    });

    info!("Initializing MutAnt layer (including Storage)...");
    let config = if cli_args.local {
        info!("Using local (Devnet) network configuration.");
        MutAntConfig {
            network: NetworkChoice::Devnet,
            ..Default::default()
        }
    } else {
        info!("Using Mainnet network configuration.");
        MutAntConfig {
            network: NetworkChoice::Mainnet,
            ..Default::default()
        }
    };

    let (mutant, mutant_init_handle) =
        match MutAnt::init_with_progress(private_key_hex, config, Some(init_callback)).await {
            Ok((a, h)) => (a, h),
            Err(e) => {
                if !init_pb.is_finished() {
                    init_pb.abandon_with_message("Initialization Failed".to_string());
                }
                // Cleanup only the mp handle as the mutant handle wasn't returned
                cleanup_background_tasks(mp_join_handle, None).await;
                return Err(CliError::MutAntInit(e.to_string()));
            }
        };

    if !init_pb.is_finished() {
        init_pb.finish_and_clear();
    }

    let exit_code = handle_command(cli_args.command, mutant, &multi_progress).await;

    info!("MutAnt CLI command finished processing.");

    cleanup_background_tasks(mp_join_handle, mutant_init_handle).await;

    Ok(exit_code)
}
