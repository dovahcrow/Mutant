use crate::index::structure::PadStatus;
use crate::network::NetworkChoice;
use crate::{index::structure::PadInfo, network::error::NetworkError};
use autonomi::client::payment::Receipt;
use autonomi::{client::payment::PaymentOption, Network, SecretKey, Wallet};
use hex;
use log::info;
use sha2::{Digest, Sha256};

pub(crate) fn create_wallet(
    private_key_hex: &str,
    network_choice: NetworkChoice,
) -> Result<(Wallet, SecretKey), NetworkError> {
    info!(
        "Creating Autonomi wallet and key for network: {:?}",
        network_choice
    );
    let network = Network::new(network_choice == NetworkChoice::Devnet)
        .map_err(|e| NetworkError::NetworkInitError(format!("Network init failed: {}", e)))?;

    let hex_to_decode = private_key_hex
        .strip_prefix("0x")
        .unwrap_or(private_key_hex);
    let pk_bytes = hex::decode(hex_to_decode)
        .map_err(|e| NetworkError::InvalidKeyInput(format!("Invalid hex private key: {}", e)))?;

    let mut hasher = Sha256::new();
    hasher.update(&pk_bytes);
    let hash_result = hasher.finalize();
    let key_array: [u8; 32] = hash_result.into();
    let secret_key = SecretKey::from_bytes(key_array).map_err(|e| {
        NetworkError::InvalidKeyInput(format!("Failed to create SecretKey from HASH: {:?}", e))
    })?;

    let wallet = Wallet::new_from_private_key(network, private_key_hex)
        .map_err(|e| NetworkError::WalletError(format!("Failed to create wallet: {}", e)))?;

    Ok((wallet, secret_key))
}

pub(super) fn payment_option_for_pad(
    pad_info: &PadInfo,
    wallet: &Wallet,
) -> Result<PaymentOption, NetworkError> {
    match pad_info.status {
        PadStatus::Generated => Ok(PaymentOption::Wallet(wallet.clone())),
        _ => {
            if let Some(receipt) = &pad_info.receipt {
                Ok(PaymentOption::Receipt(receipt.clone()))
            } else {
                Err(NetworkError::InternalError(format!(
                    "Pad status is {:?}, but no receipt found",
                    pad_info.status
                )))
            }
        }
    }
}
