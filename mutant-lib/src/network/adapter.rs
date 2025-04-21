use crate::index::structure::PadStatus;
use crate::network::client::create_client;
use crate::network::error::NetworkError;
use crate::network::wallet::create_wallet;
use crate::network::NetworkChoice;

use autonomi::client::payment::PaymentOption;
use autonomi::client::quote::{CostError, DataTypes, StoreQuote};
use autonomi::scratchpad::ScratchpadError;
use autonomi::AttoTokens;
use autonomi::{Bytes, Client, Scratchpad, ScratchpadAddress, SecretKey, Wallet, XorName};
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
    /// Uses `PaymentOption::Wallet` or `PaymentOption::Receipt` depending on context.
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

    /// Updates an existing scratchpad on the network using its secret key.
    ///
    /// This assumes the scratchpad already exists and was paid for.
    /// It uses the underlying `client.scratchpad_update` which is documented as free.
    /// The client method handles fetching the current pad, updating content, incrementing counter, and re-signing.
    ///
    /// # Arguments
    ///
    /// * `owner_key` - The secret key of the scratchpad to update.
    /// * `data_encoding` - The encoding type identifier for the data.
    /// * `data` - The new unencrypted data to store.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if the client cannot be initialized or if the update fails.
    pub async fn scratchpad_update(
        &self,
        owner_key: &SecretKey,
        data_encoding: u64,
        data: &Bytes,
    ) -> Result<(), NetworkError> {
        let addr = ScratchpadAddress::new(owner_key.public_key());
        trace!(
            "AutonomiNetworkAdapter::scratchpad_update called for address: {} with encoding {}",
            addr,
            data_encoding
        );
        let client = self.get_or_init_client().await?;

        client
            .scratchpad_update(owner_key, data_encoding, data)
            .await
            .map_err(|e| {
                error!("Failed to update scratchpad {}: {}", addr, e);
                // Adjust error mapping based on available ScratchpadError variants
                match e {
                    // If the network layer itself returns an error, wrap it.
                    ScratchpadError::Network(client_err) => NetworkError::InternalError(format!(
                        "Network error during scratchpad_update for {}: {}",
                        addr, client_err
                    )),
                    // Treat other specific scratchpad errors as internal network errors
                    // as they likely indicate issues finding/accessing the pad on the network.
                    _ => NetworkError::InternalError(format!(
                        "Failed to update scratchpad {}: {}",
                        addr, e
                    )),
                }
            })
    }

    /// Fetches storage quotes for the given content addresses and sizes.
    /// Wraps the `Client::get_store_quotes` method.
    ///
    /// # Arguments
    ///
    /// * `data_type` - The type of data being stored (e.g., `DataTypes::Scratchpad`).
    /// * `content_addrs` - An iterator providing tuples of `(XorName, usize)` representing
    ///                     the address and size of each content chunk.
    ///
    /// # Errors
    ///
    /// Returns `NetworkError` if the client cannot be initialized or if fetching quotes fails.
    pub async fn get_store_quotes(
        &self,
        data_type: DataTypes,
        content_addrs: impl Iterator<Item = (XorName, usize)>,
    ) -> Result<StoreQuote, NetworkError> {
        trace!(
            "AutonomiNetworkAdapter::get_store_quotes called for data type {:?}",
            data_type
        );
        let client = self.get_or_init_client().await?;

        // Collect into a Vec because the underlying client method might require it
        // or if the iterator needs to be consumed multiple times (though it shouldn't here).
        // If the client method truly accepts any iterator, this collect can be removed.
        let addrs_vec: Vec<(XorName, usize)> = content_addrs.collect();
        let addrs_count = addrs_vec.len();

        client
            .get_store_quotes(data_type, addrs_vec.into_iter())
            .await
            .map_err(|e: CostError| {
                error!(
                    "Failed to get store quotes for {} addresses: {}",
                    addrs_count, e
                );
                // Map CostError to NetworkError. Need to decide on the variant.
                // InternalError seems appropriate as it's a failure in a required network op.
                NetworkError::InternalError(format!(
                    "Failed to get store quotes for {} addresses: {}",
                    addrs_count, e
                ))
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
