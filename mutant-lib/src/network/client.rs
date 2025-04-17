use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::Client;
use log::info;

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
