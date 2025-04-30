pub use crate::error::Error;

mod get;
mod health_check;
mod purge;
mod put;
mod sync;
mod utils;
pub mod worker;

use crate::{
    events::{GetCallback, PurgeCallback, SyncCallback},
    index::master_index::MasterIndex,
    network::Network,
};
use autonomi::ScratchpadAddress;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

use mutant_protocol::{
    HealthCheckCallback, HealthCheckResult, PurgeResult, PutCallback, StorageMode, SyncResult,
};

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const PAD_RECYCLING_RETRIES: usize = 3;
const WORKER_COUNT: usize = 10;
pub const BATCH_SIZE: usize = 10;

const MAX_CONFIRMATION_DURATION: Duration = Duration::from_secs(60 * 20);

pub struct Data {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
}

impl Data {
    pub fn new(network: Arc<Network>, index: Arc<RwLock<MasterIndex>>) -> Self {
        Self { network, index }
    }

    pub async fn put(
        &self,
        key_name: &str,
        content: Arc<Vec<u8>>,
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        put::put(
            self.index.clone(),
            self.network.clone(),
            key_name,
            content,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await
    }

    pub async fn get_public(
        &self,
        address: &ScratchpadAddress,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        get::get_public(self.network.clone(), address, get_callback).await
    }

    pub async fn get(
        &self,
        name: &str,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        get::get(self.index.clone(), self.network.clone(), name, get_callback).await
    }

    pub async fn purge(
        &self,
        aggressive: bool,
        purge_callback: Option<PurgeCallback>,
    ) -> Result<PurgeResult, Error> {
        purge::purge(
            self.index.clone(),
            self.network.clone(),
            aggressive,
            purge_callback,
        )
        .await
    }

    pub async fn health_check(
        &self,
        key_name: &str,
        recycle: bool,
        health_check_callback: Option<HealthCheckCallback>,
    ) -> Result<HealthCheckResult, Error> {
        health_check::health_check(
            self.index.clone(),
            self.network.clone(),
            key_name,
            recycle,
            health_check_callback,
        )
        .await
    }

    pub async fn sync(
        &self,
        force: bool,
        sync_callback: Option<SyncCallback>,
    ) -> Result<SyncResult, Error> {
        sync::sync(
            self.index.clone(),
            self.network.clone(),
            force,
            sync_callback,
        )
        .await
    }
}

// fn derive_master_index_info(
//     private_key_hex: &str,
// ) -> Result<(ScratchpadAddress, SecretKey), Error> {
//     debug!("Deriving master index key and address...");
//     let hex_to_decode = private_key_hex
//         .strip_prefix("0x")
//         .unwrap_or(private_key_hex);

//     let input_key_bytes = hex::decode(hex_to_decode)
//         .map_err(|e| Error::Config(format!("Failed to decode private key hex: {}", e)))?;

//     let mut hasher = Sha256::new();
//     hasher.update(&input_key_bytes);
//     let hash_result = hasher.finalize();
//     let key_array: [u8; 32] = hash_result.into();

//     let derived_key = SecretKey::from_bytes(key_array)
//         .map_err(|e| Error::Internal(format!("Failed to create SecretKey from HASH: {:?}", e)))?;
//     let derived_public_key = derived_key.public_key();
//     let address = ScratchpadAddress::new(derived_public_key);
//     info!("Derived Master Index Address: {}", address);
//     Ok((address, derived_key))
// }
