use crate::index::structure::PadStatus;
use crate::network::client::create_client;
use crate::network::error::NetworkError;
use crate::network::wallet::create_wallet;
use crate::network::NetworkChoice;

use autonomi::client::payment::PaymentOption;
use autonomi::scratchpad::ScratchpadError;
use autonomi::AttoTokens;
use autonomi::{Bytes, Client, Scratchpad, ScratchpadAddress, SecretKey, Wallet};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Provides an interface to interact with the Autonomi network.
///
/// This adapter handles client initialization, wallet management, and interactions
/// like reading and writing scratchpads.
pub struct AutonomiNetworkAdapter {
    wallet: Arc<Wallet>,
    network_choice: NetworkChoice,
    client: OnceCell<Arc<Client>>,
}

impl AutonomiNetworkAdapter {
    /// Creates a new `AutonomiNetworkAdapter` instance.
    ///
    /// Initializes the wallet based on the provided private key and network choice.
    /// The network client is initialized lazily on first use.
    ///
    /// # Arguments
    ///
    /// * `private_key_hex` - The private key in hexadecimal format.
    /// * `network_choice` - The specific Autonomi network to connect to (e.g., Mainnet, Testnet).
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if wallet creation fails.
    pub fn new(private_key_hex: &str, network_choice: NetworkChoice) -> Result<Self, NetworkError> {
        debug!(
            "Creating AutonomiNetworkAdapter configuration for network: {:?}",
            network_choice
        );

        let (wallet, _key) = create_wallet(private_key_hex, network_choice)?;

        Ok(Self {
            wallet: Arc::new(wallet),
            network_choice,
            client: OnceCell::new(),
        })
    }

    /// Retrieves the underlying Autonomi network client, initializing it if necessary.
    ///
    /// This method ensures the client is created only once and reused for subsequent calls.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if client initialization fails.
    pub async fn get_or_init_client(&self) -> Result<Arc<Client>, NetworkError> {
        self.client
            .get_or_try_init(|| async {
                let _wallet_clone = Arc::clone(&self.wallet);
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
    ///
    /// # Arguments
    ///
    /// * `address` - The address of the scratchpad to retrieve.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if the client cannot be initialized or if fetching the scratchpad fails.
    pub async fn get_raw_scratchpad(
        &self,
        address: &ScratchpadAddress,
    ) -> Result<Scratchpad, NetworkError> {
        trace!(
            "AutonomiNetworkAdapter::get_raw_scratchpad called for address: {}",
            address
        );
        let client = self.get_or_init_client().await?;

        let scratchpad: Scratchpad = client
            .scratchpad_get(address)
            .await
            .map_err(|e| NetworkError::InternalError(format!("Failed to get scratchpad: {}", e)))?;

        Ok(scratchpad)
    }

    /// Puts raw data onto the Autonomi network, either creating a new scratchpad or updating an existing one.
    ///
    /// The behavior depends on the `current_status` of the pad:
    /// - `Generated`: Attempts to create a new scratchpad.
    /// - `Allocated`, `Written`, `Confirmed`: Attempts to update the existing scratchpad.
    ///
    /// # Arguments
    ///
    /// * `key` - The secret key associated with the scratchpad.
    /// * `data` - The raw byte data to be stored.
    /// * `current_status` - The last known status of the pad, used to determine whether to create or update.
    /// * `content_type` - The content type of the data being stored.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if:
    /// - The client cannot be initialized.
    /// - The network operation (create or update) fails.
    /// - An inconsistent state is detected (e.g., trying to create an existing pad or update a non-existent one).
    pub async fn put_raw(
        &self,
        key: &SecretKey,
        data: &[u8],
        current_status: &PadStatus,
        content_type: u64,
    ) -> Result<ScratchpadAddress, NetworkError> {
        trace!(
            "AutonomiNetworkAdapter::put_raw called, data_len: {}, status: {:?}, content_type: {}",
            data.len(),
            current_status,
            content_type
        );
        let client = self.get_or_init_client().await?;

        let public_key = key.public_key();
        let address = ScratchpadAddress::new(public_key);
        let data_bytes = Bytes::copy_from_slice(data);
        let payment_option = PaymentOption::Wallet((*self.wallet).clone());

        match current_status {
            PadStatus::Generated => {
                debug!(
                    "Attempting to create scratchpad {} (status: Generated)",
                    address
                );

                match client
                    .scratchpad_create(key, content_type, &data_bytes, payment_option.clone())
                    .await
                {
                    Ok((_cost, created_addr)) => {
                        if created_addr != address {
                            warn!(
                                "Scratchpad create returned address {} but expected {}",
                                created_addr, address
                            );
                        }
                        info!("Successfully created new scratchpad at {}", address);
                        Ok(address)
                    }
                    Err(e) if matches!(e, ScratchpadError::ScratchpadAlreadyExists(_)) => {
                        trace!(
                            "Create failed for pad {} (already exists), attempting update...",
                            address
                        );
                        match client
                            .scratchpad_update(key, content_type, &data_bytes)
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    "Successfully updated scratchpad {} after initial create failed (already exists).",
                                    address
                                );
                                Ok(address)
                            }
                            Err(update_err) => {
                                error!(
                                    "Failed to update scratchpad {} after create failed: {}",
                                    address, update_err
                                );
                                Err(NetworkError::InternalError(format!(
                                    "Failed to update scratchpad {} after create failed: {}",
                                    address, update_err
                                )))
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to create scratchpad {}: {}", address, e);
                        Err(NetworkError::InternalError(format!(
                            "Failed to create scratchpad {}: {}",
                            address, e
                        )))
                    }
                }
            }
            PadStatus::Allocated | PadStatus::Written | PadStatus::Confirmed => {
                if *current_status == PadStatus::Confirmed {
                    warn!(
                        "Calling put_raw for a pad already marked as Confirmed: {}",
                        address
                    );
                }
                debug!(
                    "Attempting to update scratchpad {} (status: {:?})",
                    address, current_status
                );
                match client
                    .scratchpad_update(key, content_type, &data_bytes)
                    .await
                {
                    Ok(_) => {
                        trace!(
                             "Executed scratchpad_update for pad {}. Data integrity depends on SDK behavior.",
                             address
                         );
                        info!("Successfully updated existing scratchpad at {}", address);
                        Ok(address)
                    }
                    Err(e) => {
                        let error_string = e.to_string().to_lowercase();
                        if error_string.contains("not found")
                            || error_string.contains("does not exist")
                        {
                            error!(
                                "Inconsistent state: Tried to update pad {}, but it does not exist (status was {:?}).",
                                address, current_status
                            );
                            Err(NetworkError::InconsistentState(format!(
                                "Update failed for {:?} pad {}: not found",
                                current_status, address
                            )))
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
    }

    /// Checks if a scratchpad exists on the network at the given address.
    ///
    /// # Arguments
    ///
    /// * `address` - The address of the scratchpad to check.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if the client cannot be initialized or if the check operation fails.
    pub async fn check_existence(&self, address: &ScratchpadAddress) -> Result<bool, NetworkError> {
        trace!(
            "AutonomiNetworkAdapter::check_existence called for address: {}",
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

    /// Returns the network choice this adapter is configured for.
    pub fn get_network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    /// Returns a reference to the underlying wallet.
    ///
    /// Note: This method is marked `#[allow(dead_code)]` as it might not be used externally
    /// but can be useful for debugging or specific internal use cases.
    #[allow(dead_code)]
    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }

    /// Puts a pre-constructed scratchpad onto the network.
    ///
    /// This is used for public uploads where the scratchpad is created
    /// with specific encoding and without encryption by the caller.
    ///
    /// # Arguments
    ///
    /// * `scratchpad` - The `Scratchpad` instance to upload.
    /// * `payment` - The payment option to use.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if the client cannot be initialized or if the upload fails.
    pub async fn scratchpad_put(
        &self,
        scratchpad: Scratchpad,
        payment: PaymentOption,
    ) -> Result<(AttoTokens, ScratchpadAddress), NetworkError> {
        let addr = *scratchpad.address(); // Get address before moving scratchpad
        trace!(
            "AutonomiNetworkAdapter::scratchpad_put called for address: {}",
            addr
        );
        let client = self.get_or_init_client().await?;

        client
            .scratchpad_put(scratchpad, payment)
            .await
            .map_err(|e| {
                error!("Failed to put scratchpad {}: {}", addr, e);
                NetworkError::InternalError(format!("Failed to put scratchpad {}: {}", addr, e))
            })
    }
}

/// Creates a new public (unencrypted) Scratchpad instance with a valid signature.
///
/// This is used for creating both public data chunks and public index scratchpads.
/// Moved here from DataManager for better separation of concerns.
pub(crate) fn create_public_scratchpad(
    owner_sk: &SecretKey,
    data_encoding: u64,
    raw_data: &Bytes,
    counter: u64,
) -> Scratchpad {
    trace!("Creating public scratchpad with encoding {}", data_encoding);
    let owner_pk = owner_sk.public_key();
    let address = ScratchpadAddress::new(owner_pk);

    // Data is passed directly as "encrypted_data" but is not actually encrypted.
    let encrypted_data = raw_data.clone();

    let bytes_to_sign =
        Scratchpad::bytes_for_signature(address, data_encoding, &encrypted_data, counter);
    let signature = owner_sk.sign(&bytes_to_sign);

    Scratchpad::new_with_signature(owner_pk, data_encoding, encrypted_data, counter, signature)
}
