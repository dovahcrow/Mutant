use crate::network::error::NetworkError;
use crate::network::NetworkChoice;
use autonomi::{Client, ClientConfig, InitialPeersConfig};
use deadpool::managed::{self, PoolError};
use thiserror::Error;
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

        // client_config.strategy.scratchpad.get_retry = RetryStrategy::Persistent;
        // client_config.strategy.scratchpad.get_quorum = ResponseQuorum::All;
        // client_config.strategy.scratchpad.verification_retry = RetryStrategy::Persistent;
        // client_config.strategy.scratchpad.verification_quorum = ResponseQuorum::All;
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
        NetworkChoice::Mainnet => config.evm_network = autonomi::Network::ArbitrumOne,
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

            let addrs = vec![
                "/ip4/206.189.96.49/udp/49841/quic-v1/p2p/12D3KooWQp3XJ6SRVLvLhezJQ7QgTQWFwDDVvwrXZQrFL4NfebWX".to_string().try_into().unwrap(),
            ];

            let config = ClientConfig {
                init_peers_config: InitialPeersConfig {
                    first: false,
                    addrs: addrs, // either provide a vec of multiaddr
                    network_contacts_url: network_contacts_url, // see other earlier list
                    local: false,
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

#[derive(Error, Debug)]
pub enum PoolManagerError {
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error("Pool interaction error: {0}")]
    Pool(String),
}

impl From<PoolError<PoolManagerError>> for PoolManagerError {
    fn from(e: PoolError<PoolManagerError>) -> Self {
        PoolManagerError::Pool(e.to_string())
    }
}

#[derive(Debug)]
pub struct ClientManager {
    pub network_choice: NetworkChoice,
    pub config: Config,
}

impl managed::Manager for ClientManager {
    type Type = Client;
    type Error = PoolManagerError;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        let network_choice = self.network_choice; // Copy values
        let config = self.config;
        info!(
            "Creating new client for pool (Network: {:?}, Config: {:?})",
            network_choice, config
        );
        create_client(network_choice, config)
            .await
            .map_err(PoolManagerError::Network)
    }

    async fn recycle(
        &self,
        conn: &mut Self::Type,
        metrics: &managed::Metrics,
    ) -> managed::RecycleResult<Self::Error> {
        // TODO: Implement a proper health check if the Client API supports it.
        // For now, assume the client is always healthy.
        // Use args to satisfy borrow checker and avoid unused warnings
        let _ = conn;
        let _ = metrics;
        Ok(())
    }
}
