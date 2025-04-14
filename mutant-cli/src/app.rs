use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use directories::{BaseDirs, ProjectDirs};
use indicatif::MultiProgress;
use log::{debug, error, info, warn};
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

use crate::callbacks::create_init_callback;
use crate::cli::{Cli, Commands};

#[derive(Serialize, Deserialize, Debug, Default)]
struct MutantCliConfig {
    wallet_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum CliError {
    WalletRead(io::Error, PathBuf),
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

// Implement conversion from the library error type
impl From<mutant_lib::error::Error> for CliError {
    fn from(lib_err: mutant_lib::error::Error) -> Self {
        // You might want more sophisticated mapping here later,
        // but for now, just wrap the display output.
        CliError::MutAntInit(lib_err.to_string()) // Reusing MutAntInit for simplicity
    }
}

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
        .map_err(|e| CliError::ConfigParse(e, config_path.to_path_buf()))?;
    fs::write(config_path, content).map_err(|e| CliError::ConfigWrite(e, config_path.to_path_buf()))
}

fn get_autonomi_wallet_dir() -> Result<PathBuf, CliError> {
    let base_dirs = BaseDirs::new().ok_or(CliError::WalletDirNotFound)?;
    let data_dir = base_dirs.data_dir();
    let wallet_dir = data_dir.join("autonomi/client/wallets");
    if wallet_dir.is_dir() {
        Ok(wallet_dir)
    } else {
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
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("0x") && name.len() > 40 {
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
            Err(CliError::WalletNotSet)
        }
    }
}

async fn initialize_wallet() -> Result<String, CliError> {
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
                config.wallet_path = None;
            }
        }

        info!("No valid wallet in config, scanning Autonomi wallet directory...");
        let wallet_dir = get_autonomi_wallet_dir()?;
        let available_wallets = scan_wallet_dir(&wallet_dir)?;

        let selected_wallet = prompt_user_for_wallet(&available_wallets)?;
        info!("Selected wallet: {:?}", selected_wallet);

        config.wallet_path = Some(selected_wallet.clone());
        save_config(&config_path, &config)?;
        info!("Saved selected wallet path to config: {:?}", config_path);
        break selected_wallet;
    };

    let private_key_hex = {
        let content = fs::read_to_string(&wallet_path)
            .map_err(|e| CliError::WalletRead(e, wallet_path.clone()))?;
        debug!("Raw content read from wallet file: '{}'", content.trim());
        content.trim().to_string()
    };
    debug!("Using private key hex from file: '{}'", private_key_hex);

    Ok(private_key_hex)
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

    let cli = Cli::parse();

    // Determine private key: Use hardcoded for local, otherwise load from wallet.
    let private_key = if cli.local {
        info!("Using hardcoded local/devnet secret key for testing.");
        // Standard Anvil/Hardhat dev key 0
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string()
    } else {
        match initialize_wallet().await {
            Ok(key) => key,
            Err(e) => {
                error!("Failed to initialize wallet: {}", e);
                return Err(e);
            }
        }
    };

    let network_choice = if cli.local {
        NetworkChoice::Devnet
    } else {
        NetworkChoice::Mainnet
    };

    let mp = MultiProgress::new();
    let _mp_clone_for_task = mp.clone();
    let (_, init_callback_fn): (_, InitCallback) = create_init_callback(&mp);

    let config = MutAntConfig {
        network: network_choice,
    };

    let mutant = MutAnt::init_with_progress(private_key, config, Some(init_callback_fn)).await?;

    let mp_join_handle = tokio::spawn(async move {
        let _keep_alive = _mp_clone_for_task;
        std::future::pending::<()>().await;
    });

    let command_result: Result<ExitCode, CliError> = match cli.command {
        Commands::Put { key, value, force } => {
            Ok(crate::commands::put::handle_put(mutant, key, value, force, &mp).await)
        }
        Commands::Get { key } => Ok(crate::commands::get::handle_get(mutant, key, &mp).await),
        Commands::Rm { key } => Ok(crate::commands::remove::handle_rm(mutant, key).await),
        Commands::Ls { long } => Ok(crate::commands::ls::handle_ls(mutant, long).await),
        Commands::Stats => Ok(crate::commands::stats::handle_stats(mutant).await),
        Commands::Reset => crate::commands::reset::handle_reset(mutant)
            .await
            .map_err(|e| CliError::MutAntInit(e.to_string())),
        Commands::Import { private_key } => {
            Ok(crate::commands::import::handle_import(mutant, private_key).await)
        }
        Commands::Sync { push_force } => {
            match crate::commands::sync::handle_sync(mutant, push_force).await {
                Ok(_) => Ok(ExitCode::SUCCESS),
                Err(e) => {
                    error!("Sync command failed: {}", e);
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Commands::Purge => {
            match crate::commands::purge::run(crate::commands::purge::PurgeArgs {}, mutant, &mp)
                .await
            {
                Ok(_) => Ok(ExitCode::SUCCESS),
                Err(e) => {
                    error!("Purge command failed: {}", e);
                    Err(CliError::MutAntInit(e.to_string()))
                }
            }
        }
    };

    let exit_code = match command_result {
        Ok(code) => code,
        Err(e) => {
            error!("Command execution failed: {}", e);
            return Err(e);
        }
    };

    info!("Command handling finished, cleaning up background tasks...");

    cleanup_background_tasks(mp_join_handle, None).await;

    info!("Cleanup complete.");

    Ok(exit_code)
}
