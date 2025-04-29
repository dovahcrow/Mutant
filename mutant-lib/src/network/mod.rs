pub mod client;
pub mod error;
pub mod get;
pub mod put;
pub mod wallet;

use blsttc::SecretKey;
use client::Config;
use deadpool::managed::{self, Pool, PoolError};
pub use error::NetworkError;

use self::client::create_client;
use self::wallet::create_wallet;
use crate::index::PadInfo;

// Make this public so other test modules can use it
pub const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

use autonomi::{AttoTokens, Client, ScratchpadAddress, Wallet};
use log::{debug, info};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NetworkChoice {
    Mainnet,
    Devnet,
    Alphanet,
}

impl Default for NetworkChoice {
    fn default() -> Self {
        NetworkChoice::Mainnet
    }
}

/// Represents the result of a get operation on the network.
#[derive(Debug, Clone)]
pub struct GetResult {
    /// The data retrieved from the scratchpad.
    pub data: Vec<u8>,
    /// The counter value of the scratchpad.
    pub counter: u64,
    /// The encoding of the data in the scratchpad.
    pub data_encoding: u64,
}

/// Represents the result of a put operation on the network.
#[derive(Debug, Clone)]
pub struct PutResult {
    /// The cost of the put operation in AttoTokens.
    pub cost: AttoTokens,
    /// The address of the scratchpad that was put.
    pub address: ScratchpadAddress,
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
    network_choice: NetworkChoice,
    config: Config,
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

/// Provides an interface to interact with the Autonomi network.
///
/// This adapter handles client initialization, wallet management, and delegates
/// network interactions like reading and writing scratchpads to specialized modules.
pub struct Network {
    wallet: Wallet,
    network_choice: NetworkChoice,
    pool: Pool<ClientManager>,
    secret_key: SecretKey,
}

impl Network {
    /// Creates a new `AutonomiNetworkAdapter` instance.
    pub(crate) fn new(
        private_key_hex: &str,
        network_choice: NetworkChoice,
    ) -> Result<Self, NetworkError> {
        debug!(
            "Creating AutonomiNetworkAdapter configuration for network: {:?}",
            network_choice
        );

        let (wallet, secret_key) = create_wallet(private_key_hex, network_choice)?;

        let manager = ClientManager {
            network_choice,
            config: Config::Get,
        };

        // get the max_connections from the env
        let max_connections = std::env::var("MAX_CONNECTIONS")
            .unwrap_or_else(|_| "20".to_string())
            .parse::<usize>()
            .unwrap_or(20);

        debug!("Max connections: {}", max_connections);

        let pool = Pool::builder(manager)
            .max_size(max_connections) // Adjust size as needed
            .build()
            .map_err(|e| {
                NetworkError::ClientInitError(format!("Failed to create GET pool: {}", e))
            })?;

        Ok(Self {
            wallet,
            network_choice,
            pool,
            secret_key,
        })
    }

    /// Retrieves an Autonomi network client from the appropriate pool.
    async fn get_client(
        &self,
        config: Config,
    ) -> Result<managed::Object<ClientManager>, PoolManagerError> {
        debug!("Requesting client from pool (Config: {:?})", config);
        self.pool.get().await.map_err(PoolManagerError::from)
    }

    /// Retrieves the raw content of a scratchpad from the network.
    /// Delegates to the `get` module.
    /// Requires PadInfo to reconstruct the SecretKey for decryption.
    pub(crate) async fn get(
        &self,
        address: &ScratchpadAddress,
        owner_sk: Option<&SecretKey>,
    ) -> Result<GetResult, NetworkError> {
        get::get(self, address, owner_sk).await
    }

    pub(crate) async fn put(
        &self,
        pad_info: &PadInfo,
        data: &[u8],
        data_encoding: u64,
        is_public: bool,
    ) -> Result<PutResult, NetworkError> {
        put::put(self, pad_info, data, data_encoding, is_public).await
    }

    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    pub fn network_choice(&self) -> NetworkChoice {
        self.network_choice
    }
}

#[cfg(test)]
pub mod integration_tests;
