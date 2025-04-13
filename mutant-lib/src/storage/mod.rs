mod init;
mod network;

use crate::error::Error;
use autonomi::{Client, ScratchpadAddress, SecretKey, Wallet};

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
    client: Client,
    master_index_address: ScratchpadAddress,
    master_index_key: SecretKey,
}

pub use init::new;

impl Storage {
    pub(crate) async fn create_scratchpad_internal_raw(
        &self,
        initial_data: &[u8],
        content_type: u64,
    ) -> Result<(ScratchpadAddress, SecretKey), Error> {
        let owner_key = SecretKey::random();
        let payment_option = autonomi::client::payment::PaymentOption::from(&self.wallet);

        let address = create_scratchpad_static(
            &self.client,
            &owner_key,
            initial_data,
            content_type,
            payment_option,
        )
        .await?;

        Ok((address, owner_key))
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn get_master_index_info(&self) -> (ScratchpadAddress, SecretKey) {
        (self.master_index_address, self.master_index_key.clone())
    }
}
