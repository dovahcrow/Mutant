use super::Storage;
use crate::error::Error;
use crate::mutant::NetworkChoice;
use autonomi::{ScratchpadAddress, SecretKey, Wallet};
use log::info;
use tokio::sync::OnceCell;

pub fn new(
    wallet: Wallet,
    network_choice: NetworkChoice,
    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey,
) -> Result<Storage, Error> {
    info!("Storage::new: Creating storage instance (lazy client initialization)");

    // No network client initialization here
    // No remote MIS loading/creation here

    let storage = Storage {
        wallet,
        client: OnceCell::new(), // Initialize as empty
        network_choice,
        master_index_address,
        master_index_key,
    };

    info!("Storage instance created successfully.");
    Ok(storage)
}
