use std::{num::NonZero, time::Duration};

use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Client, ClientConfig, ResponseQuorum, RetryStrategy};
use log::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Config {
    Get,
    Put,
}

fn get_config(config: Config) -> ClientConfig {
    let mut client_config = ClientConfig::default();

    if config == Config::Get {
        // client_config.strategy.scratchpad.get_retry = RetryStrategy::N(NonZero::new(1).unwrap());
        // client_config.strategy.scratchpad.get_quorum = ResponseQuorum::One;

        client_config.strategy.scratchpad.get_retry = RetryStrategy::Balanced;
        client_config.strategy.scratchpad.get_quorum = ResponseQuorum::Majority;
    } else {
        client_config.strategy.scratchpad.get_retry = RetryStrategy::Balanced;
        client_config.strategy.scratchpad.get_quorum = ResponseQuorum::Majority;

        client_config.strategy.scratchpad.verification_retry = RetryStrategy::Balanced;
        client_config.strategy.scratchpad.verification_quorum = ResponseQuorum::Majority;
        client_config.strategy.scratchpad.put_retry = RetryStrategy::Persistent;
        client_config.strategy.scratchpad.put_quorum = ResponseQuorum::All;
    }

    client_config
}

pub(crate) async fn create_client(
    network_choice: NetworkChoice,
    config: Config,
) -> Result<Client, NetworkError> {
    info!(
        "Initializing Autonomi network connection for {:?}...",
        network_choice
    );

    let mut config = get_config(config);

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
