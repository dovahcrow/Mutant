use crate::network::client::create_client;
use crate::network::error::NetworkError;
use crate::network::wallet::create_wallet;
use crate::network::NetworkChoice;
use async_trait::async_trait;
use autonomi::client::payment::PaymentOption; // Added specific import
use autonomi::{Client, Scratchpad, ScratchpadAddress, SecretKey, Wallet}; // Removed PaymentOption
use log::{debug, trace};
use std::sync::Arc;

/// Trait defining the interface for low-level network operations related to scratchpads.
/// This abstracts the underlying network implementation (e.g., autonomi client).
#[async_trait]
pub trait NetworkAdapter: Send + Sync {
    /// Fetches raw data from a scratchpad address.
    async fn get_raw(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, NetworkError>;

    /// Puts raw data into a scratchpad address using the provided secret key.
    /// Puts raw data into a scratchpad address. The implementation uses the adapter's internal wallet for signing.
    async fn put_raw(&self, address: &ScratchpadAddress, data: &[u8]) -> Result<(), NetworkError>;

    /// Deletes a scratchpad entry using its address and secret key.
    async fn delete_raw(
        &self,
        address: &ScratchpadAddress,
        key: &SecretKey,
    ) -> Result<(), NetworkError>;

    /// Returns the network choice (Devnet/Mainnet) the adapter is configured for.
    fn get_network_choice(&self) -> NetworkChoice;

    /// Provides access to the underlying wallet instance.
    /// Use with caution; prefer methods that abstract wallet interactions.
    fn wallet(&self) -> &Wallet;

    // Consider adding:
    // async fn get_client(&self) -> Result<autonomi::Client, NetworkError>; // If direct client access is truly needed, but ideally avoided.
}

// --- Implementation ---

/// Concrete implementation of NetworkAdapter using the autonomi crate.
#[derive(Clone)] // Clone is cheap due to Arc
pub struct AutonomiNetworkAdapter {
    wallet: Wallet, // Wallet might be needed for more than just client creation later
    client: Arc<Client>,
    network_choice: NetworkChoice, // Store for quick access
}

impl AutonomiNetworkAdapter {
    /// Creates a new AutonomiNetworkAdapter instance.
    pub async fn new(
        private_key_hex: &str,
        network_choice: NetworkChoice,
    ) -> Result<Self, NetworkError> {
        debug!(
            "Creating AutonomiNetworkAdapter for network: {:?}",
            network_choice
        );
        let wallet = create_wallet(private_key_hex, network_choice)?;
        let client = create_client(wallet.clone()).await?; // Clone wallet for client creation
        Ok(Self {
            wallet,
            client: Arc::new(client),
            network_choice,
        })
    }
}

#[async_trait]
impl NetworkAdapter for AutonomiNetworkAdapter {
    async fn get_raw(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, NetworkError> {
        trace!("NetworkAdapter::get_raw called for address: {}", address);
        let scratchpad = self
            .client
            .scratchpad_get(address)
            .await
            .map_err(NetworkError::AutonomiClient)?;
        // Assuming Scratchpad has a field like .data to get Vec<u8>
        // If this fails, we'll need to adjust based on the actual Scratchpad structure.
        Ok(scratchpad.data) // Changed to field access .data
    }

    async fn put_raw(&self, address: &ScratchpadAddress, data: &[u8]) -> Result<(), NetworkError> {
        trace!(
            "NetworkAdapter::put_raw called for address: {}, data_len: {}",
            address,
            data.len()
        );
        // Re-attempting Scratchpad construction based on E0308 hint
        let owner_pk = self.wallet.pk(); // Changed to pk()
        let nonce = 0; // Assuming 0 is acceptable for nonce
                       // Assuming signature Scratchpad::new(address, owner_pk, nonce, content: Vec<u8>)
                       // This might still be wrong if the signature is different (e.g., expects &Bytes)
        let scratchpad = Scratchpad::new(*address, owner_pk, nonce, data.to_vec());
        let payment = PaymentOption::PayForUpload;

        self.client
            .scratchpad_put(scratchpad, payment)
            .await
            .map_err(NetworkError::AutonomiClient)
            .map(|(_cost, _addr)| ()) // Discard the cost and address, return () on success
    }

    async fn delete_raw(
        &self,
        address: &ScratchpadAddress,
        _key: &SecretKey, // Key might not be needed if client uses wallet
    ) -> Result<(), NetworkError> {
        trace!("NetworkAdapter::delete_raw called for address: {}", address);
        // TODO: Implement delete_raw using available autonomi client methods if possible.
        // The client.scratchpad_delete method seems to be missing in v0.4.3.
        // For now, return an error or unimplemented!()
        Err(NetworkError::InternalError(
            "delete_raw is not implemented for AutonomiNetworkAdapter".to_string(),
        ))
        // self.client
        //     .scratchpad_delete(address, key) // This method doesn't seem to exist
        //     .await
        //     .map_err(NetworkError::AutonomiClient)
    }

    fn get_network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}
