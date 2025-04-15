use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Network, Wallet};
use hex;
use log::info;

// Creates an Autonomi wallet from a private key hex string.
pub(crate) fn create_wallet(
    private_key_hex: &str,
    network_choice: NetworkChoice,
) -> Result<Wallet, NetworkError> {
    info!("Creating Autonomi wallet for network: {:?}", network_choice);
    let network = Network::new(network_choice == NetworkChoice::Devnet)
        .map_err(|e| NetworkError::NetworkInitError(format!("Network init failed: {}", e)))?;

    // Handle potential "0x" prefix
    let hex_to_decode = if private_key_hex.starts_with("0x") {
        &private_key_hex[2..]
    } else {
        private_key_hex
    };

    // Basic validation before decoding
    if hex_to_decode.len() != 64 {
        // 32 bytes * 2 hex chars/byte
        return Err(NetworkError::InvalidKeyInput(format!(
            "Invalid private key hex length. Expected 64 characters, got {}",
            hex_to_decode.len()
        )));
    }

    Wallet::new_from_private_key(network, hex_to_decode)
        .map_err(|e| NetworkError::WalletError(format!("Failed to create wallet: {}", e)))
}
