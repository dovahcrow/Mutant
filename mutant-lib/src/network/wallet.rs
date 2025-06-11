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

    // Generate a new random private key for testnet use
    // This ensures the key is valid for secp256k1 and unique each time
    let new_private_key_hex = derive_testnet_private_key(master_private_key_hex)?;
    info!("Generated new random private key for testnet");

    // Create a wallet from this private key
    let new_wallet = Wallet::new_from_private_key(network.clone(), &new_private_key_hex)
        .map_err(|e| NetworkError::WalletError(format!("Failed to create wallet from generated private key: {}", e)))?;
    let new_address = new_wallet.address();
    let new_address_hex = format!("0x{:x}", new_address);

    info!("Created new wallet with address: {}", new_address_hex);

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
        let amount_str = "10000000000000000000"; // 1000 * 10^18
        amount_str.parse().unwrap_or(master_token_balance.saturating_sub(master_token_balance))
    } else {
        master_token_balance.saturating_sub(master_token_balance) // Zero
    };

    let transfer_gas = if !master_gas_balance.is_zero() {
        // Transfer 1000 gas tokens (with 18 decimals = 1000 * 10^18)
        let amount_str = "10000000000000000000"; // 1000 * 10^18
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

    info!("Generated testnet wallet with public address: {}", new_address_hex);

    Ok((new_wallet, new_private_key_hex, new_address_hex))
}

/// Generates a new valid secp256k1 private key for testnet use
/// This ensures the generated key is random but valid for EVM operations
fn derive_testnet_private_key(_master_private_key_hex: &str) -> Result<String, NetworkError> {
    use rand::RngCore;

    // Generate random bytes and add timestamp for uniqueness
    let mut rng = rand::thread_rng();
    let mut random_bytes = [0u8; 24]; // 24 random bytes
    rng.fill_bytes(&mut random_bytes);

    // Add current timestamp for uniqueness on each call
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Combine random bytes + timestamp for entropy
    let mut hasher = Sha256::new();
    hasher.update(&random_bytes);
    hasher.update(&timestamp.to_le_bytes());
    hasher.update(b"testnet_unique_key"); // Salt for domain separation
    let derived_hash = hasher.finalize();

    // secp256k1 curve order (n) - private key must be in range [1, n-1]
    // n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
    let secp256k1_order = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
        0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B,
        0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41
    ];

    // Ensure the derived key is within valid range by taking modulo (n-1) and adding 1
    let mut derived_key_bytes = derived_hash.to_vec();

    // Simple modular reduction: if derived >= n, subtract n until < n
    while is_greater_or_equal(&derived_key_bytes, &secp256k1_order) {
        subtract_bytes(&mut derived_key_bytes, &secp256k1_order);
    }

    // Ensure key is not zero (must be >= 1)
    if derived_key_bytes.iter().all(|&b| b == 0) {
        derived_key_bytes[31] = 1; // Set to 1 if somehow zero
    }

    let derived_private_key_hex = format!("0x{}", hex::encode(derived_key_bytes));
    info!("Successfully generated valid secp256k1 private key for testnet");

    Ok(derived_private_key_hex)
}

/// Helper function to compare if a >= b for 32-byte arrays
fn is_greater_or_equal(a: &[u8], b: &[u8]) -> bool {
    for i in 0..32 {
        if a[i] > b[i] {
            return true;
        } else if a[i] < b[i] {
            return false;
        }
    }
    true // Equal case
}

/// Helper function to subtract b from a (a = a - b) for 32-byte arrays
fn subtract_bytes(a: &mut [u8], b: &[u8]) {
    let mut borrow = 0u16;
    for i in (0..32).rev() {
        let temp = a[i] as u16 + 256 - b[i] as u16 - borrow;
        a[i] = temp as u8;
        borrow = if temp < 256 { 1 } else { 0 };
    }
}
