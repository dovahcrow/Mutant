pub mod client;
pub mod error;
pub mod get;
pub mod put;
pub mod wallet;

use blsttc::SecretKey;
pub use error::NetworkError;

use self::client::create_client;
use self::wallet::create_wallet;
use crate::index::structure::PadInfo;

use autonomi::client::payment::Receipt;
use autonomi::{AttoTokens, Client, Scratchpad, ScratchpadAddress, Wallet};
use log::{debug, info};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Re-export NetworkChoice for easier access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NetworkChoice {
    Mainnet,
    Devnet,
}

pub struct GetResult {
    pub data: Vec<u8>,
    pub counter: u64,
    pub data_encoding: u64,
}

#[derive(Debug, Clone)]
pub struct PutResult {
    pub cost: AttoTokens,
    pub address: ScratchpadAddress,
    pub counter: u64,
    pub data_encoding: u64,
    pub receipt: Receipt,
    pub size_tmp: usize,
}

/// Provides an interface to interact with the Autonomi network.
///
/// This adapter handles client initialization, wallet management, and delegates
/// network interactions like reading and writing scratchpads to specialized modules.
pub struct AutonomiNetworkAdapter {
    wallet: Arc<Wallet>,
    network_choice: NetworkChoice,
    client: OnceCell<Arc<Client>>,
    secret_key: SecretKey,
}

impl AutonomiNetworkAdapter {
    /// Creates a new `AutonomiNetworkAdapter` instance.
    pub fn new(private_key_hex: &str, network_choice: NetworkChoice) -> Result<Self, NetworkError> {
        debug!(
            "Creating AutonomiNetworkAdapter configuration for network: {:?}",
            network_choice
        );

        let (wallet, secret_key) = create_wallet(private_key_hex, network_choice)?;

        Ok(Self {
            wallet: Arc::new(wallet),
            network_choice,
            client: OnceCell::new(),
            secret_key,
        })
    }

    /// Retrieves the underlying Autonomi network client, initializing it if necessary.
    /// (Moved from old adapter.rs)
    async fn get_or_init_client(&self) -> Result<Arc<Client>, NetworkError> {
        self.client
            .get_or_try_init(|| async {
                let network_choice_clone = self.network_choice;
                info!(
                    "Initializing network client for {:?}...",
                    network_choice_clone
                );
                create_client(network_choice_clone).await.map(Arc::new)
            })
            .await
            .map(Arc::clone)
    }

    /// Retrieves the raw content of a scratchpad from the network.
    /// Delegates to the `get` module.
    /// Requires PadInfo to reconstruct the SecretKey for decryption.
    pub async fn get_private(&self, pad_info: &PadInfo) -> Result<GetResult, NetworkError> {
        // Reconstruct SecretKey from bytes stored in PadInfo
        let owner_sk = pad_info.secret_key();
        // Pass Some(secret_key) to get::get
        get::get(self, &pad_info.address, Some(&owner_sk)).await
    }

    pub async fn get_public(&self, address: &ScratchpadAddress) -> Result<GetResult, NetworkError> {
        // Pass None to get::get as no decryption is needed
        get::get(self, address, None).await
    }

    pub async fn put_private(
        &self,
        pad_info: &PadInfo,
        data: &[u8],
        data_encoding: u64,
    ) -> Result<PutResult, NetworkError> {
        self.put(pad_info, data, data_encoding, false).await
    }

    pub async fn put_public(
        &self,
        pad_info: &PadInfo,
        data: &[u8],
        data_encoding: u64,
    ) -> Result<PutResult, NetworkError> {
        println!("putting public");
        self.put(pad_info, data, data_encoding, true).await
    }

    /// Puts a scratchpad onto the network using `scratchpad_put`.
    /// Delegates to the `put` module.
    async fn put(
        &self,
        pad_info: &PadInfo,
        data: &[u8],
        data_encoding: u64,
        is_public: bool,
    ) -> Result<PutResult, NetworkError> {
        put::put(self, pad_info, data, data_encoding, is_public).await
    }

    pub async fn fetch_receipt(&self, pad_info: &PadInfo) -> Result<Receipt, NetworkError> {
        println!(
            "fetching receipt {:#?}, {:#?}",
            pad_info.address, pad_info.size
        );
        let client = self.get_or_init_client().await?;
        let quotes = client
            .get_store_quotes(
                autonomi::client::quote::DataTypes::Scratchpad,
                vec![(pad_info.address.xorname(), pad_info.size)].into_iter(),
            )
            .await
            .map_err(|e| NetworkError::InternalError(format!("Failed to fetch receipt: {}", e)))?;

        println!("quotes: {:#?}", quotes.len());

        let receipt = autonomi::client::payment::receipt_from_store_quotes(quotes);

        println!("receipt: {:#?}", receipt);

        Ok(receipt)
    }

    // /// Returns the network choice this adapter is configured for.
    // pub fn get_network_choice(&self) -> NetworkChoice {
    //     self.network_choice
    // }
}

impl Default for NetworkChoice {
    fn default() -> Self {
        NetworkChoice::Mainnet // Default to Mainnet
    }
}

#[cfg(test)]
pub mod integration_tests;
