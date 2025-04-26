use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Client, ClientConfig, InitialPeersConfig, ResponseQuorum, RetryStrategy};
use log::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Config {
    Get,
    Put,
}

fn get_config(config: Config) -> ClientConfig {
    let client_config = ClientConfig::default();

    if config == Config::Get {
        // client_config.strategy.scratchpad.get_retry = RetryStrategy::N(NonZero::new(1).unwrap());
        // client_config.strategy.scratchpad.get_quorum = ResponseQuorum::One;

        // client_config.strategy.scratchpad.get_retry = RetryStrategy::Balanced;
        // client_config.strategy.scratchpad.get_quorum = ResponseQuorum::Majority;
    } else {
        // client_config.strategy.scratchpad.get_retry = RetryStrategy::Balanced;
        // client_config.strategy.scratchpad.get_quorum = ResponseQuorum::Majority;

        // client_config.strategy.scratchpad.verification_retry = RetryStrategy::Balanced;
        // client_config.strategy.scratchpad.verification_quorum = ResponseQuorum::Majority;
        // client_config.strategy.scratchpad.put_retry = RetryStrategy::Persistent;
        // client_config.strategy.scratchpad.put_quorum = ResponseQuorum::All;
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
        NetworkChoice::Alphanet => {}
    };

    let client = match network_choice {
        NetworkChoice::Mainnet => Client::init_with_config(config).await,
        NetworkChoice::Devnet => Client::init_local().await,
        NetworkChoice::Alphanet => {
            let network_contacts_url = vec![
                "http://174.138.6.129/bootstrap_cache.json".to_string(),
                "http://206.189.7.202/bootstrap_cache.json".to_string(),
                "http://146.190.225.26/bootstrap_cache.json".to_string(),
                "http://164.90.207.31/bootstrap_cache.json".to_string(),
                "http://178.62.197.211/bootstrap_cache.json".to_string(),
            ];
            let peers_multi_addr = vec![];
            let config = ClientConfig {
                init_peers_config: InitialPeersConfig {
                    first: false,
                    addrs: peers_multi_addr, // either provide a vec of multiaddr
                    network_contacts_url: network_contacts_url, // see other earlier list
                    local: false,
                    disable_mainnet_contacts: true,
                    ignore_cache: false,
                    bootstrap_cache_dir: None,
                },
                evm_network: autonomi::Network::ArbitrumSepoliaTest,
                strategy: Default::default(),
                network_id: Some(2),
            };
            Client::init_with_config(config).await
        }
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
