use crate::error::Error;
use crate::index::master_index::MasterIndex;
use crate::network::Network;
use autonomi::{Client, ScratchpadAddress};
use blsttc::SecretKey;
use log::{debug, info};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const PAD_RECYCLING_RETRIES: usize = 3;
pub const WORKER_COUNT: usize = 20;
pub const BATCH_SIZE: usize = 10;

pub const MAX_CONFIRMATION_DURATION: Duration = Duration::from_secs(60 * 20);

#[derive(Clone)]
pub struct Context {
    pub index: Arc<RwLock<MasterIndex>>,
    pub network: Arc<Network>,
    pub name: Arc<String>,
    pub chunks: Arc<Vec<Vec<u8>>>,
}

pub fn derive_master_index_info(
    private_key_hex: &str,
) -> Result<(ScratchpadAddress, SecretKey), Error> {
    debug!("Deriving master index key and address...");
    let hex_to_decode = private_key_hex
        .strip_prefix("0x")
        .unwrap_or(private_key_hex);

    let input_key_bytes = hex::decode(hex_to_decode)
        .map_err(|e| Error::Config(format!("Failed to decode private key hex: {}", e)))?;

    let mut hasher = Sha256::new();
    hasher.update(&input_key_bytes);
    let hash_result = hasher.finalize();
    let key_array: [u8; 32] = hash_result.into();

    let derived_key = SecretKey::from_bytes(key_array)
        .map_err(|e| Error::Internal(format!("Failed to create SecretKey from HASH: {:?}", e)))?;
    let derived_public_key = derived_key.public_key();
    let address = ScratchpadAddress::new(derived_public_key);
    info!("Derived Master Index Address: {}", address);
    Ok((address, derived_key))
}
