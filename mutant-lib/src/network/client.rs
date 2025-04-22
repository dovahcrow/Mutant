use std::time::Duration;

use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Client, ClientConfig, RetryStrategy};
use log::{debug, info};

pub(crate) async fn create_client(network_choice: NetworkChoice) -> Result<Client, NetworkError> {
    info!(
        "Initializing Autonomi network connection for {:?}...",
        network_choice
    );

    let mut config = ClientConfig::default();
    // config.strategy.scratchpad.verification_retry = RetryStrategy::Persistent;
    config.strategy.scratchpad.put_retry = RetryStrategy::Persistent;
    // config.strategy.scratchpad.get_retry = RetryStrategy::Persistent;

    match network_choice {
        NetworkChoice::Mainnet => config.evm_network = autonomi::Network::new(false).unwrap(),
        NetworkChoice::Devnet => config.evm_network = autonomi::Network::new(true).unwrap(),
    };

    let client = match network_choice {
        NetworkChoice::Mainnet => Client::init_with_config(config).await,
        NetworkChoice::Devnet => Client::init_local().await,
    };

    let client = client.map_err(|e| {
        NetworkError::ClientInitError(format!("Failed to initialize Autonomi client: {}", e))
    })?;

    info!(
        "Autonomi client initialized successfully for {:?}.",
        network_choice
    );

    Ok(client)
}
