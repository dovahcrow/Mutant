use std::{collections::BTreeMap, sync::Arc};

use autonomi::ScratchpadAddress;
use tokio::sync::RwLock;

use crate::{
    error::Error,
    events::{GetCallback, PurgeCallback, SyncCallback},
    index::{
        master_index::{IndexEntry, MasterIndex, StorageStats},
        PadInfo,
    },
    network::{Network, NetworkChoice, DEV_TESTNET_PRIVATE_KEY_HEX},
    ops::Data,
};

use mutant_protocol::{
    HealthCheckCallback, HealthCheckResult, PurgeResult, PutCallback, StorageMode, SyncResult,
};

/// The main entry point for interacting with the MutAnt distributed storage system.
///
/// This struct encapsulates the different managers (data, index, pad lifecycle) and the network adapter.
/// Instances are typically created using the `init` or `init_with_progress` associated functions.
#[derive(Clone)]
pub struct MutAnt {
    index: Arc<RwLock<MasterIndex>>,
    data: Arc<RwLock<Data>>,
}

impl MutAnt {
    async fn init_all(private_key_hex: &str, network_choice: NetworkChoice) -> Result<Self, Error> {
        let network = Arc::new(Network::new(private_key_hex, network_choice)?);
        let index = Arc::new(RwLock::new(MasterIndex::new(network_choice)));
        let data = Arc::new(RwLock::new(Data::new(network.clone(), index.clone())));

        Ok(Self { index, data })
    }
    pub async fn init(private_key_hex: &str) -> Result<Self, Error> {
        Self::init_all(private_key_hex, NetworkChoice::Mainnet).await
    }

    pub async fn init_public() -> Result<Self, Error> {
        Self::init_all(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Mainnet).await
    }

    pub async fn init_local() -> Result<Self, Error> {
        Self::init_all(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet).await
    }

    pub async fn init_public_local() -> Result<Self, Error> {
        Self::init_all(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet).await
    }

    pub async fn init_alphanet(private_key_hex: &str) -> Result<Self, Error> {
        Self::init_all(private_key_hex, NetworkChoice::Alphanet).await
    }

    pub async fn init_public_alphanet() -> Result<Self, Error> {
        Self::init_all(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Alphanet).await
    }

    pub async fn put(
        &self,
        user_key: &str,
        data_bytes: Arc<Vec<u8>>,
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        self.data
            .read()
            .await
            .put(user_key, data_bytes, mode, public, no_verify, put_callback)
            .await
    }

    pub async fn get(
        &self,
        user_key: &str,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        self.data.read().await.get(user_key, get_callback).await
    }

    pub async fn get_public(
        &self,
        address: &ScratchpadAddress,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        self.data
            .read()
            .await
            .get_public(address, get_callback)
            .await
    }

    pub async fn rm(&self, user_key: &str) -> Result<(), Error> {
        self.index.write().await.remove_key(user_key)?;
        Ok(())
    }

    pub async fn list(&self) -> Result<BTreeMap<String, IndexEntry>, Error> {
        let keys = self.index.read().await.list();
        Ok(keys)
    }

    pub async fn export_raw_pads_private_key(&self) -> Result<Vec<PadInfo>, Error> {
        let pads_hex = self.index.read().await.export_raw_pads_private_key()?;
        Ok(pads_hex)
    }

    pub async fn import_raw_pads_private_key(&self, pads_hex: Vec<PadInfo>) -> Result<(), Error> {
        self.index
            .write()
            .await
            .import_raw_pads_private_key(pads_hex)?;

        Ok(())
    }

    pub async fn purge(
        &self,
        aggressive: bool,
        purge_callback: Option<PurgeCallback>,
    ) -> Result<PurgeResult, Error> {
        self.data
            .read()
            .await
            .purge(aggressive, purge_callback)
            .await
    }

    pub async fn get_storage_stats(&self) -> StorageStats {
        self.index.read().await.get_storage_stats()
    }

    pub async fn health_check(
        &self,
        key_name: &str,
        recycle: bool,
        health_check_callback: Option<HealthCheckCallback>,
    ) -> Result<HealthCheckResult, Error> {
        self.data
            .read()
            .await
            .health_check(key_name, recycle, health_check_callback)
            .await
    }

    pub async fn sync(
        &self,
        force: bool,
        sync_callback: Option<SyncCallback>,
    ) -> Result<SyncResult, Error> {
        self.data.read().await.sync(force, sync_callback).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .put(
                &user_key,
                Arc::new(data_bytes),
                StorageMode::Medium,
                false,
                false,
                None,
            )
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());
        // Ideally, we'd also check if the data is retrievable here,
        // but that requires a `get` method which is not yet implemented.
        // For now, we just check if the store operation completed without error.

        // check of the index
        let keys = mutant.list().await.unwrap();
        assert!(keys.contains_key(&user_key));

        // check of the data
        let data = mutant.get(&user_key, None).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_update() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant
            .put(
                &user_key,
                Arc::new(data_bytes),
                StorageMode::Medium,
                false,
                false,
                None,
            )
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data_bytes = generate_random_bytes(128);

        let result = mutant
            .put(
                &user_key,
                Arc::new(data_bytes),
                StorageMode::Medium,
                false,
                false,
                None,
            )
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data = mutant.get(&user_key, None).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_resume() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        // Start the first put operation but do not await its completion
        let first_put = mutant.put(
            &user_key,
            Arc::new(data_bytes),
            StorageMode::Medium,
            false,
            false,
            None,
        );

        // Simulate an interruption by dropping the future before it completes
        drop(first_put);

        // Now attempt to resume the operation with the same data
        let result = mutant
            .put(
                &user_key,
                Arc::new(data_bytes),
                StorageMode::Medium,
                false,
                false,
                None,
            )
            .await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        // Verify that the data is correctly stored
        let data = mutant.get(&user_key, None).await.unwrap();
        assert_eq!(data, data_bytes);
    }
}
