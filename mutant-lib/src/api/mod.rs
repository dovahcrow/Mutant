// use crate::api::init::initialize_layers;
// use crate::api::ReserveCallback;
// use crate::data::manager::DefaultDataManager;
// use crate::index::manager::DefaultIndexManager;
// use crate::index::structure::{IndexEntry, MasterIndex};
// use crate::internal_error::Error;
// use crate::internal_events::{GetCallback, InitCallback, PurgeCallback, PutCallback};
// use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
// use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
// use crate::types::{KeyDetails, KeySummary, MutAntConfig, StorageStats};
// use autonomi::{Bytes, ScratchpadAddress, SecretKey};
// use log::{debug, info, warn};
// use std::collections::HashSet;
// use std::sync::Arc;

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::{
    data::Data,
    index::master_index::{KeysInfo, MasterIndex},
    internal_error::Error,
    network::{Network, NetworkChoice},
};

/// The main entry point for interacting with the MutAnt distributed storage system.
///
/// This struct encapsulates the different managers (data, index, pad lifecycle) and the network adapter.
/// Instances are typically created using the `init` or `init_with_progress` associated functions.
#[derive(Clone)]
pub struct MutAnt {
    // data_manager: Arc<DefaultDataManager>,
    // pad_lifecycle_manager: Arc<DefaultPadLifecycleManager>,
    // index_manager: Arc<DefaultIndexManager>,
    // network: Network,
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
    data: Arc<Data>,
}

impl MutAnt {
    pub async fn init(private_key_hex: &str) -> Result<Self, Error> {
        let network = Arc::new(Network::new(private_key_hex, NetworkChoice::Mainnet)?);
        let index = Arc::new(RwLock::new(MasterIndex::new()));

        let data = Arc::new(Data::new(network.clone(), index.clone()));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn store(&self, user_key: &str, data_bytes: &[u8]) -> Result<(), Error> {
        self.data.store(user_key, data_bytes).await
    }

    // pub async fn store_public(&self, user_key: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

    // pub async fn get(&self, user_key: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn get_public(&self, user_key: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, user_key: &[u8]) -> Result<(), Error> {}

    // pub async fn reserve_pads(&self, count: usize) -> Result<usize, Error> {}

    // pub async fn sync(&self, force: bool) -> Result<(), Error> {}

    pub async fn list(&self) -> Result<KeysInfo, Error> {
        let keys = self.index.read().await.list();

        Ok(KeysInfo { keys })
    }
}
