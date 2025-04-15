use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::Client;
use log::info;

// Placeholder for client creation logic.
// This might handle lazy initialization or specific client configurations.
pub(crate) async fn create_client(network_choice: NetworkChoice) -> Result<Client, NetworkError> {
    info!(
        "Initializing Autonomi network connection for {:?}...",
        network_choice
    );

    let client_result = match network_choice {
        NetworkChoice::Mainnet => Client::init().await,
        NetworkChoice::Devnet => Client::init_local().await,
    };

    let client = client_result.map_err(|e| {
        NetworkError::ClientInitError(format!("Failed to initialize Autonomi client: {}", e))
    })?;

    info!(
        "Autonomi client initialized successfully for {:?}.",
        network_choice
    );
    Ok(client)
}

// Placeholder for retry logic if needed directly on client operations.
// Example:
// pub(crate) async fn get_with_retry(...) -> ... { ... }
