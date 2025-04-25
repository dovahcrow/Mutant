use dialoguer::{theme::ColorfulTheme, Select};
use directories::{BaseDirs, ProjectDirs};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::error::Error;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct WalletConfig {
    pub wallet_path: Option<PathBuf>,
}

fn get_autonomi_wallet_dir() -> Result<PathBuf, Error> {
    let base_dirs = BaseDirs::new().ok_or(Error::WalletDirNotFound)?;
    let data_dir = base_dirs.data_dir();
    let wallet_dir = data_dir.join("autonomi/client/wallets");
    if wallet_dir.is_dir() {
        Ok(wallet_dir)
    } else {
        warn!(
            "Standard Autonomi wallet directory not found at {:?}",
            wallet_dir
        );
        Err(Error::WalletDirNotFound)
    }
}

fn scan_wallet_dir(wallet_dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries =
        fs::read_dir(wallet_dir).map_err(|e| Error::WalletDirRead(e, wallet_dir.to_path_buf()))?;
    let mut wallets = Vec::new();
    for entry_result in entries {
        let entry = entry_result.map_err(|e| Error::WalletDirRead(e, wallet_dir.to_path_buf()))?;
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
        Err(Error::NoWalletsFound(wallet_dir.to_path_buf()))
    } else {
        Ok(wallets)
    }
}

fn prompt_user_for_wallet(wallets: &[PathBuf]) -> Result<PathBuf, Error> {
    if wallets.is_empty() {
        return Err(Error::WalletNotSet);
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
        .map_err(Error::UserSelectionFailed)?;

    match selection {
        Some(index) => Ok(wallets[index].clone()),
        None => {
            error!("No wallet selected by the user.");
            Err(Error::WalletNotSet)
        }
    }
}

pub async fn scan_and_select_wallet() -> Result<String, Error> {
    info!("Scanning Autonomi wallet directory...");
    let wallet_dir = get_autonomi_wallet_dir()?;
    let available_wallets = scan_wallet_dir(&wallet_dir)?;
    let selected_wallet = prompt_user_for_wallet(&available_wallets)?;
    info!("Selected wallet: {:?}", selected_wallet);

    let private_key_hex = {
        let content = fs::read_to_string(&selected_wallet)
            .map_err(|e| Error::WalletRead(e, selected_wallet.clone()))?;
        debug!("Raw content read from wallet file: '{}'", content.trim());
        content.trim().to_string()
    };
    debug!("Using private key hex from file: '{}'", private_key_hex);

    Ok(private_key_hex)
}
