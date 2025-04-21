#![cfg(test)]

use crate::data::{PRIVATE_DATA_ENCODING, PUBLIC_DATA_ENCODING};
use crate::index::structure::{PadInfo, PadStatus};
use crate::network::{AutonomiNetworkAdapter, NetworkChoice, NetworkError};
use crate::pad_lifecycle::PadOrigin;
use autonomi::{ScratchpadAddress, SecretKey};
use rand::RngCore; // For generating random data

const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn setup_adapter() -> AutonomiNetworkAdapter {
    AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
        .expect("Test adapter setup failed")
}

fn generate_random_data(size: usize) -> Vec<u8> {
    let mut data = vec![0u8; size];
    rand::thread_rng().fill_bytes(&mut data);
    data
}

#[tokio::test]
async fn test_get_public() {}

#[tokio::test]
async fn test_get_private() {}

#[tokio::test]
async fn test_put_public() {}

#[tokio::test]
async fn test_put_private() {}

#[tokio::test]
async fn test_fetch_receipt() {}
