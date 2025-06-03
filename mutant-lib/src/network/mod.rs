pub mod client;
pub mod error;
pub mod get;
pub mod put;
pub mod wallet;

use blsttc::SecretKey;
use client::Config;
pub use error::NetworkError;

use self::wallet::create_wallet;
use crate::index::PadInfo;

// Make this public so other test modules can use it
pub const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

use autonomi::{AttoTokens, Client, ScratchpadAddress, Wallet};
use log::debug;

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

lazy_static::lazy_static! {
    pub static ref NB_CLIENTS: usize = std::env::var("MUTANT_NB_CLIENTS")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<usize>()
        .unwrap_or(10);

    pub static ref BATCH_SIZE: usize = std::env::var("MUTANT_BATCH_SIZE")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<usize>()
        .unwrap_or(10);
}

/// Provides an interface to interact with the Autonomi network.
///
/// This adapter handles client initialization, wallet management, and delegates
/// network interactions like reading and writing scratchpads to specialized modules.
pub struct Network {
    wallet: Wallet,
    network_choice: NetworkChoice,
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

        Ok(Self {
            wallet,
            network_choice,
            secret_key,
        })
    }

    /// Retrieves an Autonomi network client.
    /// This method creates a new client for each call.
    pub(crate) async fn get_client(
        &self,
        config: Config,
    ) -> Result<Client, NetworkError> {
        client::create_client(self.network_choice, config).await
    }

    /// Retrieves the raw content of a scratchpad from the network.
    /// Delegates to the `get` module.
    /// Requires PadInfo to reconstruct the SecretKey for decryption.
    pub(crate) async fn get<C: std::ops::Deref<Target = Client>>(
        &self,
        client: C,
        address: &ScratchpadAddress,
        owner_sk: Option<&SecretKey>,
    ) -> Result<GetResult, NetworkError> {
        get::get(client.deref(), address, owner_sk).await
    }

    pub(crate) async fn put<C: std::ops::Deref<Target = Client>>(
        &self,
        client: C,
        pad_info: &PadInfo,
        data: &[u8],
        data_encoding: u64,
        is_public: bool,
    ) -> Result<PutResult, NetworkError> {
        put::put(
            client.deref(),
            self.wallet.clone(),
            pad_info,
            data,
            data_encoding,
            is_public,
        )
        .await
    }

    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    pub fn network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}

#[cfg(test)]
pub mod integration_tests;
