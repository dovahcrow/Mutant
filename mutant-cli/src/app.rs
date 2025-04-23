use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use directories::{BaseDirs, ProjectDirs};
use indicatif::{MultiProgress, ProgressDrawTarget};
use log::{debug, error, info, warn};

// use mutant_lib::config::MutAntConfig;
// use mutant_lib::error::Error as LibError;
use mutant_lib::{MutAnt, config::NetworkChoice, error::Error as LibError};

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

// use crate::callbacks::create_init_callback;
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
    UserInputAborted(String),
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
            CliError::UserInputAborted(msg) => write!(f, "Operation aborted by user: {}", msg),
        }
    }
}

impl std::error::Error for CliError {}

impl From<LibError> for CliError {
    fn from(lib_err: LibError) -> Self {
        CliError::MutAntInit(lib_err.to_string())
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

    let wallet_path = if let Some(ref path) = config.wallet_path {
        if path.exists() {
            info!("Using wallet from config: {:?}", path);
            path.clone()
        } else {
            warn!(
                "Wallet path from config {:?} does not exist. Rescanning.",
                path
            );
            config.wallet_path = None;
            info!("No valid wallet in config, scanning Autonomi wallet directory...");
            let wallet_dir = get_autonomi_wallet_dir()?;
            let available_wallets = scan_wallet_dir(&wallet_dir)?;
            let selected_wallet = prompt_user_for_wallet(&available_wallets)?;
            info!("Selected wallet: {:?}", selected_wallet);
            config.wallet_path = Some(selected_wallet.clone());
            save_config(&config_path, &config)?;
            info!("Saved selected wallet path to config: {:?}", config_path);
            selected_wallet
        }
    } else {
        info!("No valid wallet in config, scanning Autonomi wallet directory...");
        let wallet_dir = get_autonomi_wallet_dir()?;
        let available_wallets = scan_wallet_dir(&wallet_dir)?;
        let selected_wallet = prompt_user_for_wallet(&available_wallets)?;
        info!("Selected wallet: {:?}", selected_wallet);
        config.wallet_path = Some(selected_wallet.clone());
        save_config(&config_path, &config)?;
        info!("Saved selected wallet path to config: {:?}", config_path);
        selected_wallet
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
    debug!("cleanup_background_tasks: Starting cleanup...");
    // Abort the MultiProgress background task
    debug!("cleanup_background_tasks: Aborting MultiProgress background task...");
    mp_join_handle.abort();
    // Do not await the handle, as it's intentionally pending and await might hang.
    // match mp_join_handle.await {
    //     Ok(_) => debug!("MultiProgress task exited normally after abort."),
    //     Err(e) if e.is_cancelled() => debug!("MultiProgress task successfully cancelled."),
    //     Err(e) => error!("MultiProgress task panicked or failed: {:?}", e),
    // }
    debug!("MultiProgress task abort signal sent.");

    debug!("cleanup_background_tasks: Checking for mutant_init_handle...");
    if let Some(handle) = mutant_init_handle {
        info!("cleanup_background_tasks: Waiting for background MutAnt/Storage task...");
        match handle.await {
            Ok(_) => {
                info!(
                    "cleanup_background_tasks: Background MutAnt/Storage task finished successfully."
                );
            }
            Err(e) => {
                if e.is_panic() {
                    error!(
                        "cleanup_background_tasks: Background MutAnt/Storage task panicked: {}",
                        e
                    );
                } else if e.is_cancelled() {
                    info!(
                        "cleanup_background_tasks: Background MutAnt/Storage task was cancelled."
                    );
                } else {
                    error!(
                        "cleanup_background_tasks: Background MutAnt/Storage task failed to join: {}",
                        e
                    );
                }
            }
        }
    } else {
        debug!("cleanup_background_tasks: No mutant_init_handle found.");
    }
    debug!("cleanup_background_tasks: Finished cleanup.");
}

pub async fn run_cli() -> Result<ExitCode, CliError> {
    let cli = Cli::parse();

    let mp = MultiProgress::new();
    let draw_target = if cli.quiet {
        ProgressDrawTarget::hidden()
    } else {
        ProgressDrawTarget::stderr_with_hz(2)
    };
    mp.set_draw_target(draw_target);

    let mp_clone = mp.clone();
    let mp_join_handle = tokio::spawn(async move {
        if mp_clone.is_hidden() {
            return;
        }
        std::future::pending::<()>().await;
    });

    let mut mutant_init_handle: Option<JoinHandle<()>> = None;

    // Determine if this is a 'get -p' command BEFORE initializing wallet
    let is_public_get = matches!(cli.command, Commands::Get { public: true, .. });

    let mutant = if is_public_get {
        // // Determine which public init function to call based on --local flag
        // let mutant_instance = if cli.local {
        //     info!("Public local get command detected, using public_local initializer.");
        //     MutAnt::init_public_local().await?
        // } else {
        //     info!("Public get command detected, using public initializer (Mainnet).");
        //     MutAnt::init_public().await?
        // };
        // mutant_instance
        unimplemented!()
    } else {
        // Proceed with wallet initialization for all other commands
        let private_key_hex = if cli.local {
            info!("Using hardcoded local/devnet secret key for testing.");
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string()
        } else {
            initialize_wallet().await?
        };

        let network_choice = if cli.local {
            NetworkChoice::Devnet
        } else {
            NetworkChoice::Mainnet
        };
        // Create config using the default derived impl
        // let mut config = MutAntConfig::default();
        // config.network = network_choice;

        // let (init_pb_opt_arc, init_callback) = create_init_callback(&mp, cli.quiet);

        // Use init_with_progress for regular initialization
        let mutant_instance = match network_choice {
            NetworkChoice::Devnet => MutAnt::init_local().await?,
            NetworkChoice::Mainnet => MutAnt::init(&private_key_hex).await?,
        };
        let _mutant_clone_for_bg = mutant_instance.clone();
        // let init_pb_clone = Arc::clone(&init_pb_opt_arc); // Clone Arc for the task

        // let init_handle = tokio::spawn(async move {
        //     info!("Starting background MutAnt/Storage initialization...");
        //     // Initialize is implicitly called by init_with_progress, just monitor the progress bar
        //     // let mut pb_guard = init_pb_clone.lock().await;
        //     if let Some(pb) = pb_guard.as_mut() {
        //         // Create a future that resolves when the progress bar is finished.
        //         // This might require a more sophisticated approach depending on how finish is signaled.
        //         // For now, let's poll the is_finished status.
        //         let pb_future = async {
        //             while !pb.is_finished() {
        //                 tokio::time::sleep(Duration::from_millis(100)).await;
        //             }
        //         };

        //         // Wait for the progress bar to finish with a timeout
        //         if tokio::time::timeout(Duration::from_secs(100), pb_future)
        //             .await
        //             .is_err()
        //         {
        //             error!("Background init progress bar timed out");
        //             // Ensure pb is still valid before calling abandon
        //             if !pb.is_finished() {
        //                 pb.abandon_with_message("Initialization timed out".to_string());
        //             }
        //         }
        //         // No need to call finish_and_clear here, it should happen when init completes
        //     } else {
        //         // Handle case where progress bar wasn't created (e.g., quiet mode)
        //         debug!("No progress bar found for background init task.");
        //         // We might need to await the actual init future if it doesn't drive the progress bar.
        //         // Assuming init_with_progress handles this internally for now.
        //     }
        //     // Drop the guard explicitly
        //     drop(pb_guard);
        //     info!("Background MutAnt/Storage monitoring task finished.");
        // });

        // Clear the main thread's reference to the progress bar immediately after spawning the monitor
        // let mut pb_main_guard = init_pb_opt_arc.lock().await;
        // if let Some(_pb) = pb_main_guard.take() {
        //     // Don't clear it here, let the initialization logic handle it
        // }
        // drop(pb_main_guard);

        // mutant_init_handle = Some(init_handle);
        mutant_instance
    };

    let result = match cli.command {
        Commands::Put {
            key,
            value,
            force,
            public,
        } => {
            crate::commands::put::handle_put(mutant, key, value, force, public, &mp, cli.quiet)
                .await
        }
        Commands::Get { key, public } => {
            crate::commands::get::handle_get(mutant, key, public, &mp, cli.quiet).await
        }
        Commands::Rm { key } => crate::commands::remove::handle_rm(mutant, key).await,
        Commands::Ls { long } => crate::commands::ls::handle_ls(mutant, long).await,
        Commands::Export { output } => crate::commands::export::handle_export(mutant, output).await,
        Commands::Import { input } => crate::commands::import::handle_import(mutant, input).await,
        // Commands::Stats => crate::commands::stats::handle_stats(mutant).await,
        // Commands::Reset => crate::commands::reset::handle_reset(mutant).await,
        // Commands::Import { private_key } => {
        //     // Import needs the live mutant instance now
        //     crate::commands::import::handle_import(mutant, private_key).await
        // }
        // Commands::Sync { push_force } => {
        //     match crate::commands::sync::handle_sync(mutant, push_force).await {
        //         Ok(_) => ExitCode::SUCCESS,
        //         Err(e) => {
        //             error!("Sync command failed: {}", e);
        //             ExitCode::FAILURE
        //         }
        //     }
        // }
        // Commands::Purge => {
        //     match crate::commands::purge::run(
        //         crate::commands::purge::PurgeArgs {},
        //         mutant,
        //         &mp,
        //         cli.quiet,
        //     )
        //     .await
        //     {
        //         Ok(_) => ExitCode::SUCCESS,
        //         Err(e) => {
        //             error!("Purge command failed: {}", e);
        //             ExitCode::FAILURE
        //         }
        //     }
        // }
        // Commands::Reserve(reserve_cmd) => match reserve_cmd.run(&mutant, &mp).await {
        //     Ok(_) => ExitCode::SUCCESS,
        //     Err(e) => {
        //         error!("Reserve command failed: {}", e);
        //         ExitCode::FAILURE
        //     }
        // },
    };
    debug!(
        "run_cli: Command handler finished with result: {:?}",
        result
    );

    debug!("run_cli: Calling cleanup_background_tasks...");
    cleanup_background_tasks(mp_join_handle, mutant_init_handle).await;
    debug!("run_cli: Returned from cleanup_background_tasks.");

    debug!("run_cli: Exiting with code: {:?}", result);
    Ok(result)
}
