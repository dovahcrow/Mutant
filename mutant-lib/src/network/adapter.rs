use crate::network::client::create_client;
use crate::network::error::NetworkError;
use crate::network::wallet::create_wallet;
use crate::network::NetworkChoice;
use async_trait::async_trait;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, Client, Scratchpad, ScratchpadAddress, SecretKey, Wallet};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Trait defining the interface for low-level network operations related to scratchpads.
/// This abstracts the underlying network implementation (e.g., autonomi client).
#[async_trait]
pub trait NetworkAdapter: Send + Sync {
    /// Fetches the full Scratchpad object from a scratchpad address.
    /// Decryption must be handled by the caller using the appropriate key and the Scratchpad's decrypt_data method.
    async fn get_raw_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, NetworkError>;

    /// Puts raw data into a scratchpad associated with the given secret key.
    /// Creates the scratchpad if it doesn't exist, updates it otherwise.
    /// If `is_new_hint` is true, it may skip existence checks and attempt creation directly.
    /// Returns the address of the scratchpad.
    async fn put_raw(
        &self,
        key: &SecretKey,
        data: &[u8],
        is_new_hint: bool,
    ) -> Result<ScratchpadAddress, NetworkError>;

    /// Checks if a scratchpad exists at the given address.
    async fn check_existence(&self, address: &ScratchpadAddress) -> Result<bool, NetworkError>;

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
pub struct AutonomiNetworkAdapter {
    wallet: Arc<Wallet>,
    network_choice: NetworkChoice,
    client: OnceCell<Arc<Client>>,
}

impl AutonomiNetworkAdapter {
    /// Creates a new AutonomiNetworkAdapter instance without connecting immediately.
    pub fn new(private_key_hex: &str, network_choice: NetworkChoice) -> Result<Self, NetworkError> {
        debug!(
            "Creating AutonomiNetworkAdapter configuration for network: {:?}",
            network_choice
        );
        // Create wallet eagerly, it's cheap and needed for PaymentOption later
        let (wallet, _key) = create_wallet(private_key_hex, network_choice)?;

        Ok(Self {
            wallet: Arc::new(wallet),
            network_choice,
            client: OnceCell::new(),
        })
    }

    /// Gets the initialized client, initializing it on first call.
    async fn get_or_init_client(&self) -> Result<Arc<Client>, NetworkError> {
        self.client
            .get_or_try_init(|| async {
                // Clone Wallet and NetworkChoice to move into the async block
                let _wallet_clone = Arc::clone(&self.wallet);
                let network_choice_clone = self.network_choice;

                info!(
                    "Initializing network client for {:?}...",
                    network_choice_clone
                );
                // Use create_client which handles Client::init/init_local
                create_client(network_choice_clone).await.map(Arc::new) // Wrap the resulting Client in Arc
            })
            .await
            .map(Arc::clone) // Clone the Arc<Client> for the caller
    }
}

#[async_trait]
impl NetworkAdapter for AutonomiNetworkAdapter {
    async fn get_raw_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, NetworkError> {
        trace!(
            "NetworkAdapter::get_raw_scratchpad called for address: {}",
            address
        );
        let client = self.get_or_init_client().await?;
        // Fetch the Scratchpad object
        let scratchpad: Scratchpad = client
            .scratchpad_get(address)
            .await
            .map_err(|e| NetworkError::InternalError(format!("Failed to get scratchpad: {}", e)))?;

        // Return the whole Scratchpad object
        Ok(scratchpad)
    }

    async fn put_raw(
        &self,
        key: &SecretKey,
        data: &[u8],
        is_new_hint: bool,
    ) -> Result<ScratchpadAddress, NetworkError> {
        trace!(
            "NetworkAdapter::put_raw called, data_len: {}, is_new_hint: {}",
            data.len(),
            is_new_hint
        );
        let client = self.get_or_init_client().await?;

        let public_key = key.public_key();
        let address = ScratchpadAddress::new(public_key);
        let data_bytes = Bytes::copy_from_slice(data);
        let content_type = 0u64;
        let payment_option = PaymentOption::Wallet((*self.wallet).clone());

        if is_new_hint {
            // If hinted as new, try creating directly.
            debug!(
                "Attempting to create new scratchpad at address: {}",
                address
            );
            match client
                .scratchpad_create(key, content_type, &data_bytes, payment_option.clone())
                .await
            {
                Ok((_cost, created_addr)) => {
                    // Sanity check address?
                    if created_addr != address {
                        warn!(
                            "Scratchpad create returned address {} but expected {}",
                            created_addr, address
                        );
                        // Use the address derived from the key anyway?
                    }
                    info!("Successfully created new scratchpad at {}", address);
                    Ok(address)
                }
                Err(e) if e.to_string().contains("already exists") => {
                    // Heuristic check for existence error (Error type might be opaque)
                    warn!(
                        "Attempted to create new scratchpad at {}, but it seems to already exist. Trying update.",
                        address
                    );
                    // Fall through to update logic
                    match client
                        .scratchpad_update(key, content_type, &data_bytes)
                        .await
                    {
                        Ok(_) => {
                            info!("Successfully updated existing scratchpad at {}", address);
                            Ok(address)
                        }
                        Err(update_err) => {
                            error!(
                                "Failed to update scratchpad {} after create conflict: {}",
                                address, update_err
                            );
                            Err(NetworkError::InternalError(format!(
                                "Create conflict then update failed for {}: {}",
                                address, update_err
                            )))
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create new scratchpad {}: {}", address, e);
                    Err(NetworkError::InternalError(format!(
                        "Failed to create scratchpad {}: {}",
                        address, e
                    )))
                }
            }
        } else {
            // If not hinted as new, directly attempt update.
            debug!(
                "Attempting to update existing scratchpad at address: {}",
                address
            );
            match client
                .scratchpad_update(key, content_type, &data_bytes)
                .await
            {
                Ok(_) => {
                    info!("Successfully updated existing scratchpad at {}", address);
                    Ok(address)
                }
                Err(e) => {
                    // Handle potential errors like "not found" from the update call
                    // The error type might be opaque, requiring string matching or specific handling
                    // based on autonomi client's error specifics.
                    if e.to_string().contains("not found") {
                        warn!("Attempted to update non-existent scratchpad {}. This might indicate an index inconsistency.", address);
                        // Should we attempt creation here? Let's try it.
                        warn!("Falling back to creating scratchpad {} after update failed (not found).", address);
                        match client
                            .scratchpad_create(key, content_type, &data_bytes, payment_option)
                            .await
                        {
                            Ok((_cost, created_addr)) => {
                                if created_addr != address {
                                    warn!(
                                             "Scratchpad create (fallback) returned address {} but expected {}",
                                             created_addr, address
                                         );
                                }
                                info!("Successfully created scratchpad {} via fallback.", address);
                                Ok(address)
                            }
                            Err(create_err) => {
                                error!(
                                    "Failed to create scratchpad {} during fallback: {}",
                                    address, create_err
                                );
                                Err(NetworkError::InternalError(format!(
                                    "Update not found, then create failed for {}: {}",
                                    address, create_err
                                )))
                            }
                        }
                    } else {
                        error!("Failed to update scratchpad {}: {}", address, e);
                        Err(NetworkError::InternalError(format!(
                            "Failed to update scratchpad {}: {}",
                            address, e
                        )))
                    }
                }
            }
        }
    }

    async fn check_existence(&self, address: &ScratchpadAddress) -> Result<bool, NetworkError> {
        trace!(
            "NetworkAdapter::check_existence called for address: {}",
            address
        );
        let client = self.get_or_init_client().await?;
        client
            .scratchpad_check_existance(address)
            .await
            .map_err(|e| {
                NetworkError::InternalError(format!("Failed to check scratchpad existence: {}", e))
            })
    }

    fn get_network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}
