use std::{collections::BTreeMap, sync::Arc};

use autonomi::ScratchpadAddress;
use tokio::sync::RwLock;

use crate::{
    data::{storage_mode::StorageMode, Data},
    events::GetCallback,
    index::{
        master_index::{IndexEntry, MasterIndex, StorageStats},
        PadInfo,
    },
    internal_error::Error,
    internal_events::PutCallback,
    network::{Network, NetworkChoice, DEV_TESTNET_PRIVATE_KEY_HEX},
};

/// The main entry point for interacting with the MutAnt distributed storage system.
///
/// This struct encapsulates the different managers (data, index, pad lifecycle) and the network adapter.
/// Instances are typically created using the `init` or `init_with_progress` associated functions.
#[derive(Clone)]
pub struct MutAnt {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
    data: Arc<RwLock<Data>>,
}

impl MutAnt {
    pub async fn init(private_key_hex: &str) -> Result<Self, Error> {
        let network_choice = NetworkChoice::Mainnet;
        let network = Arc::new(Network::new(private_key_hex, network_choice)?);
        let index = Arc::new(RwLock::new(MasterIndex::new(network_choice)));

        let data = Arc::new(RwLock::new(Data::new(
            network.clone(),
            index.clone(),
            None,
            None,
        )));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn init_public() -> Result<Self, Error> {
        let network_choice = NetworkChoice::Mainnet;
        let network = Arc::new(Network::new(DEV_TESTNET_PRIVATE_KEY_HEX, network_choice)?);
        let index = Arc::new(RwLock::new(MasterIndex::new(network_choice)));
        let data = Arc::new(RwLock::new(Data::new(
            network.clone(),
            index.clone(),
            None,
            None,
        )));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn init_local() -> Result<Self, Error> {
        let network = Arc::new(Network::new(
            DEV_TESTNET_PRIVATE_KEY_HEX,
            NetworkChoice::Devnet,
        )?);
        let index = Arc::new(RwLock::new(MasterIndex::new(NetworkChoice::Devnet)));
        let data = Arc::new(RwLock::new(Data::new(
            network.clone(),
            index.clone(),
            None,
            None,
        )));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn init_public_local() -> Result<Self, Error> {
        let network = Arc::new(Network::new(
            DEV_TESTNET_PRIVATE_KEY_HEX,
            NetworkChoice::Devnet,
        )?);
        let index = Arc::new(RwLock::new(MasterIndex::new(NetworkChoice::Devnet)));
        let data = Arc::new(RwLock::new(Data::new(
            network.clone(),
            index.clone(),
            None,
            None,
        )));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn set_put_callback(&mut self, callback: PutCallback) {
        self.data.write().await.set_put_callback(callback);
    }

    pub async fn set_get_callback(&mut self, callback: GetCallback) {
        self.data.write().await.set_get_callback(callback);
    }

    pub async fn put(
        &self,
        user_key: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
    ) -> Result<ScratchpadAddress, Error> {
        self.data
            .write()
            .await
            .put(user_key, data_bytes, mode, public)
            .await
    }

    // pub async fn store_public(&self, user_key: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

    pub async fn get(&self, user_key: &str) -> Result<Vec<u8>, Error> {
        self.data.write().await.get(user_key).await
    }

    pub async fn get_public(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, Error> {
        self.data.write().await.get_public(address).await
    }

    // pub async fn get_public(&self, user_key: &[u8]) -> Result<Vec<u8>, Error> {}

    pub async fn rm(&self, user_key: &str) -> Result<(), Error> {
        self.index.write().await.remove_key(user_key)?;
        Ok(())
    }

    // pub async fn reserve_pads(&self, count: usize) -> Result<usize, Error> {}

    // pub async fn sync(&self, force: bool) -> Result<(), Error> {}

    pub async fn list(&self) -> Result<BTreeMap<String, IndexEntry>, Error> {
        let keys = self.index.read().await.list();
        Ok(keys)
    }

    pub async fn export_raw_pads_private_key(&self) -> Result<Vec<PadInfo>, Error> {
        let pads_hex = self.index.read().await.export_raw_pads_private_key()?;
        Ok(pads_hex)
    }

    pub async fn import_raw_pads_private_key(
        &mut self,
        pads_hex: Vec<PadInfo>,
    ) -> Result<(), Error> {
        self.index
            .write()
            .await
            .import_raw_pads_private_key(pads_hex)?;

        Ok(())
    }

    pub async fn purge(&self, aggressive: bool) -> Result<(), Error> {
        self.data.write().await.purge(aggressive).await
    }

    pub async fn get_storage_stats(&self) -> StorageStats {
        self.index.read().await.get_storage_stats()
    }

    pub async fn health_check(&self, key_name: &str) -> Result<(), Error> {
        self.data.write().await.health_check(key_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::DEV_TESTNET_PRIVATE_KEY_HEX;
    use rand::{distributions::Alphanumeric, Rng};

    fn generate_random_string(len: usize) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(len)
            .map(char::from)
            .collect()
    }

    fn generate_random_bytes(len: usize) -> Vec<u8> {
        let mut vec = vec![0u8; len];
        rand::thread_rng().fill(&mut vec[..]);
        vec
    }

    async fn setup_mutant() -> MutAnt {
        MutAnt::init_local()
            .await
            .expect("Failed to initialize MutAnt for test")
    }

    #[tokio::test]
    async fn test_store_basic() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant
            .put(&user_key, &data_bytes, StorageMode::Medium, false)
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());
        // Ideally, we'd also check if the data is retrievable here,
        // but that requires a `get` method which is not yet implemented.
        // For now, we just check if the store operation completed without error.

        // check of the index
        let keys = mutant.list().await.unwrap();
        assert!(keys.contains_key(&user_key));

        // check of the data
        let data = mutant.get(&user_key).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_update() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant
            .put(&user_key, &data_bytes, StorageMode::Medium, false)
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data_bytes = generate_random_bytes(128);

        let result = mutant
            .put(&user_key, &data_bytes, StorageMode::Medium, false)
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data = mutant.get(&user_key).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_resume() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        // Start the first put operation but do not await its completion
        let first_put = mutant.put(&user_key, &data_bytes, StorageMode::Medium, false);

        // Simulate an interruption by dropping the future before it completes
        drop(first_put);

        // Now attempt to resume the operation with the same data
        let result = mutant
            .put(&user_key, &data_bytes, StorageMode::Medium, false)
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        // Verify that the data is correctly stored
        let data = mutant.get(&user_key).await.unwrap();
        assert_eq!(data, data_bytes);
    }
}
