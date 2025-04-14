mod init;
pub(crate) mod network;

use crate::error::Error;
use crate::mutant::NetworkChoice;
use autonomi::{Client, ScratchpadAddress, SecretKey, Wallet};
use log::info;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

use network::create_scratchpad_static;

pub(crate) use network::fetch_remote_master_index_storage_static;
pub(crate) use network::fetch_scratchpad_internal_static;
pub(crate) use network::load_master_index_storage_static;
pub(crate) use network::storage_create_mis_from_arc_static;
pub(crate) use network::storage_save_mis_from_arc_static;
pub(crate) use network::update_scratchpad_internal_static_with_progress;

#[repr(u64)]
pub(super) enum ContentType {
    MasterIndex = 1,
    DataChunk = 2,
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

    pub(crate) fn wallet(&self) -> &Wallet {
        &self.wallet
    }

    pub(crate) fn get_master_index_info(&self) -> (ScratchpadAddress, SecretKey) {
        (self.master_index_address, self.master_index_key.clone())
    }

    pub(crate) fn get_network_choice(&self) -> NetworkChoice {
        self.network_choice
    }

    /// Loads the master index from the network, creating and saving a default one if necessary.
    pub(crate) async fn load_or_create_master_index(
        &self,
    ) -> Result<
        std::sync::Arc<tokio::sync::Mutex<crate::mutant::data_structures::MasterIndexStorage>>,
        Error,
    > {
        let client = self.get_client().await?;
        let (address, key) = self.get_master_index_info();
        // Call the internal network function (already re-exported in this mod)
        load_master_index_storage_static(client, &address, &key).await
    }
}
