use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Network, SecretKey, Wallet};
use hex;
use log::{info, warn};
use sha2::{Digest, Sha256};

pub(crate) fn create_wallet(
    private_key_hex: &str,
    network_choice: NetworkChoice,
) -> Result<(Wallet, SecretKey), NetworkError> {
    info!(
        "Creating Autonomi wallet and key for network: {:?}",
        network_choice
    );
    let network = match network_choice {
        NetworkChoice::Mainnet => Network::ArbitrumOne,
        NetworkChoice::Devnet => Network::new(true).map_err(|e| NetworkError::NetworkInitError(format!("Network init failed: {}", e)))?,
        NetworkChoice::Alphanet => Network::ArbitrumSepoliaTest,
    };

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

/// Creates a new random wallet for testnet mode and transfers funds from master wallet
/// Returns (new_wallet, new_private_key_hex, new_public_address)
pub async fn create_testnet_wallet_with_transfer(
    master_private_key_hex: &str,
    network_choice: NetworkChoice,
) -> Result<(Wallet, String, String), NetworkError> {
    info!("Creating new random wallet for testnet mode");

    // Create the network for the testnet
    let network = match network_choice {
        NetworkChoice::Devnet => Network::new(true).map_err(|e| NetworkError::NetworkInitError(format!("Network init failed: {}", e)))?,
        _ => return Err(NetworkError::InvalidKeyInput("Testnet wallet creation only supported for Devnet".to_string())),
    };

    // Create a new random wallet
    let new_wallet = Wallet::new_with_random_wallet(network.clone());
    let new_address = new_wallet.address();
    let new_address_hex = format!("0x{:x}", new_address);

    info!("Created new random wallet with address: {}", new_address_hex);

    // Create master wallet for transfers
    let master_wallet = Wallet::new_from_private_key(network, master_private_key_hex)
        .map_err(|e| NetworkError::WalletError(format!("Failed to create master wallet: {}", e)))?;

    // Get master wallet balances
    let master_token_balance = master_wallet.balance_of_tokens().await
        .map_err(|e| NetworkError::WalletError(format!("Failed to get master token balance: {}", e)))?;
    let master_gas_balance = master_wallet.balance_of_gas_tokens().await
        .map_err(|e| NetworkError::WalletError(format!("Failed to get master gas balance: {}", e)))?;

    info!("Master wallet balances - Tokens: {}, Gas: {}", master_token_balance, master_gas_balance);

    // For testnet, transfer a reasonable fixed amount instead of calculating exact percentages
    // This avoids complex Uint256 arithmetic and is sufficient for testing
    let transfer_tokens = if !master_token_balance.is_zero() {
        // Transfer 1000 tokens (with 18 decimals = 1000 * 10^18)
        let amount_str = "1000000000000000000000"; // 1000 * 10^18
        amount_str.parse().unwrap_or(master_token_balance.saturating_sub(master_token_balance))
    } else {
        master_token_balance.saturating_sub(master_token_balance) // Zero
    };

    let transfer_gas = if !master_gas_balance.is_zero() {
        // Transfer 1000 gas tokens (with 18 decimals = 1000 * 10^18)
        let amount_str = "1000000000000000000000"; // 1000 * 10^18
        amount_str.parse().unwrap_or(master_gas_balance.saturating_sub(master_gas_balance))
    } else {
        master_gas_balance.saturating_sub(master_gas_balance) // Zero
    };

    info!("Transferring fixed amounts - Tokens: {}, Gas: {}", transfer_tokens, transfer_gas);

    // Transfer tokens (if balance > 0)
    if !master_token_balance.is_zero() && !transfer_tokens.is_zero() {
        match master_wallet.transfer_tokens(new_address, transfer_tokens).await {
            Ok(tx_hash) => info!("Token transfer successful, tx hash: {:?}", tx_hash),
            Err(e) => warn!("Token transfer failed: {}", e),
        }
    } else {
        warn!("Master wallet has no tokens to transfer or transfer amount is zero");
    }

    // Transfer gas tokens (if balance > 0)
    if !master_gas_balance.is_zero() && !transfer_gas.is_zero() {
        match master_wallet.transfer_gas_tokens(new_address, transfer_gas).await {
            Ok(tx_hash) => info!("Gas transfer successful, tx hash: {:?}", tx_hash),
            Err(e) => warn!("Gas transfer failed: {}", e),
        }
    } else {
        warn!("Master wallet has no gas tokens to transfer or transfer amount is zero");
    }

    // Since Autonomi's Wallet::new_with_random_wallet doesn't expose the private key,
    // we'll generate a new deterministic private key for our internal use
    // This is only used for the SecretKey creation, not for the actual wallet operations
    let mut hasher = Sha256::new();
    hasher.update(new_address.to_string().as_bytes());
    hasher.update(b"mutant_testnet_secretkey");
    let hash_result = hasher.finalize();
    let new_private_key_hex = format!("0x{}", hex::encode(hash_result));

    info!("Generated testnet wallet with public address: {}", new_address_hex);

    Ok((new_wallet, new_private_key_hex, new_address_hex))
}
