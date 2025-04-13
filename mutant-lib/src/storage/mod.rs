mod init;
mod network;

use crate::error::Error;
use crate::mutant::NetworkChoice;
use autonomi::{Client, ScratchpadAddress, SecretKey, Wallet};
use log::info;
use tokio::sync::OnceCell;

use network::create_scratchpad_static;

pub(crate) use network::fetch_remote_master_index_storage_static;
pub(crate) use network::fetch_scratchpad_internal_static;
pub(crate) use network::storage_save_mis_from_arc_static;
pub(crate) use network::update_scratchpad_internal_static;

#[repr(u64)]
pub(super) enum ContentType {
    MasterIndex = 1,
}

#[derive(Clone)]
pub struct Storage {
    wallet: Wallet,
    client: OnceCell<Client>,
    network_choice: NetworkChoice,
    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey,
}

pub use init::new;

impl Storage {
    pub(crate) async fn get_client(&self) -> Result<&Client, Error> {
        self.client
            .get_or_try_init(|| async {
                info!(
                    "Initializing Autonomi client for network: {:?}",
                    self.network_choice
                );
                match self.network_choice {
                    NetworkChoice::Devnet => Client::init_local()
                        .await
                        .map_err(|e| Error::NetworkConnectionFailed(e.to_string())),
                    NetworkChoice::Mainnet => Client::init()
                        .await
                        .map_err(|e| Error::NetworkConnectionFailed(e.to_string())),
                }
            })
            .await
    }

    pub(crate) async fn create_scratchpad_internal_raw(
        &self,
        initial_data: &[u8],
        content_type: u64,
    ) -> Result<(ScratchpadAddress, SecretKey), Error> {
        let client = self.get_client().await?;
        let owner_key = SecretKey::random();
        let payment_option = autonomi::client::payment::PaymentOption::from(&self.wallet);

        let address = create_scratchpad_static(
            client,
            &owner_key,
            initial_data,
            content_type,
            payment_option,
        )
        .await?;

        Ok((address, owner_key))
    }

    pub(crate) fn get_master_index_info(&self) -> (ScratchpadAddress, SecretKey) {
        (self.master_index_address, self.master_index_key.clone())
    }
}
