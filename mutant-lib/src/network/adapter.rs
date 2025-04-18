use crate::index::structure::PadStatus;
use crate::network::client::create_client;
use crate::network::error::NetworkError;
use crate::network::wallet::create_wallet;
use crate::network::NetworkChoice;

use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, Client, Scratchpad, ScratchpadAddress, SecretKey, Wallet};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use tokio::sync::OnceCell;

pub struct AutonomiNetworkAdapter {
    wallet: Arc<Wallet>,
    network_choice: NetworkChoice,
    client: OnceCell<Arc<Client>>,
}

impl AutonomiNetworkAdapter {
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

    async fn get_or_init_client(&self) -> Result<Arc<Client>, NetworkError> {
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

    pub async fn put_raw(
        &self,
        key: &SecretKey,
        data: &[u8],
        current_status: &PadStatus,
    ) -> Result<ScratchpadAddress, NetworkError> {
        trace!(
            "AutonomiNetworkAdapter::put_raw called, data_len: {}, status: {:?}",
            data.len(),
            current_status
        );
        let client = self.get_or_init_client().await?;

        let public_key = key.public_key();
        let address = ScratchpadAddress::new(public_key);
        let data_bytes = Bytes::copy_from_slice(data);
        let content_type = 0u64;
        let payment_option = PaymentOption::Wallet((*self.wallet).clone());

        match current_status {
            PadStatus::Generated => {
                debug!(
                    "Attempting to create scratchpad {} (status: Generated)",
                    address
                );
                match client
                    .scratchpad_create(key, content_type, &data_bytes, payment_option)
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
                    Err(e) if e.to_string().contains("already exists") => {
                        error!(
                            "Inconsistent state: Tried to create pad {}, but it already exists (status was Generated).",
                            address
                        );
                        Err(NetworkError::InconsistentState(format!(
                            "Create failed for Generated pad {}: already exists",
                            address
                        )))
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
                        warn!(
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

    pub fn get_network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    #[allow(dead_code)]
    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}
