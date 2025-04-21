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
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
    data: Arc<Data>,
}

impl MutAnt {
    pub async fn init(private_key_hex: &str) -> Result<Self, Error> {
        let network = Arc::new(Network::new(private_key_hex, NetworkChoice::Devnet)?);
        let index = Arc::new(RwLock::new(MasterIndex::new()));

        let data = Arc::new(Data::new(network.clone(), index.clone()));

        Ok(Self {
            network,
            index,
            data,
        })
    }

    pub async fn put(&self, user_key: &str, data_bytes: &[u8]) -> Result<(), Error> {
        self.data.put(user_key, data_bytes).await
    }

    // pub async fn store_public(&self, user_key: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

    pub async fn get(&self, user_key: &str) -> Result<Vec<u8>, Error> {
        self.data.get(user_key).await
    }

    // pub async fn get_public(&self, user_key: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, user_key: &[u8]) -> Result<(), Error> {}

    // pub async fn reserve_pads(&self, count: usize) -> Result<usize, Error> {}

    // pub async fn sync(&self, force: bool) -> Result<(), Error> {}

    pub async fn list(&self) -> Result<KeysInfo, Error> {
        let keys = self.index.read().await.list();

        Ok(KeysInfo { keys })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::integration_tests::DEV_TESTNET_PRIVATE_KEY_HEX;
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
        MutAnt::init(DEV_TESTNET_PRIVATE_KEY_HEX)
            .await
            .expect("Failed to initialize MutAnt for test")
    }

    #[tokio::test]
    async fn test_store_basic() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant.put(&user_key, &data_bytes).await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());
        // Ideally, we'd also check if the data is retrievable here,
        // but that requires a `get` method which is not yet implemented.
        // For now, we just check if the store operation completed without error.

        // check of the index
        let keys = mutant.list().await.unwrap();
        assert!(keys.keys.contains(&user_key));

        // check of the data
        let data = mutant.get(&user_key).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_update() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant.put(&user_key, &data_bytes).await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data_bytes = generate_random_bytes(128);

        let result = mutant.put(&user_key, &data_bytes).await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data = mutant.get(&user_key).await.unwrap();
        assert_eq!(data, data_bytes);
    }

    #[tokio::test]
    async fn test_store_resume() {
        let mutant = setup_mutant().await;
        let user_key = generate_random_string(10);
        let data_bytes = generate_random_bytes(128);

        let result = mutant.put(&user_key, &data_bytes).await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());

        let data_bytes = generate_random_bytes(128);

        let result = mutant.put(&user_key, &data_bytes).await;

        assert!(result.is_ok(), "Store operation failed: {:?}", result.err());
    }
}
