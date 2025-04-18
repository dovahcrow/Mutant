#![cfg(test)]

use crate::index::structure::PadStatus;
use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::{NetworkChoice, NetworkError};
use crate::storage::error::StorageError;
use crate::storage::manager::DefaultStorageManager;
use autonomi::{ScratchpadAddress, SecretKey};
use std::sync::Arc;

const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn setup_storage_manager() -> DefaultStorageManager {
    let network_adapter = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test adapter setup failed"),
    );
    DefaultStorageManager::new(network_adapter)
}

#[tokio::test]
async fn test_write_read_cycle() {
    let storage_manager = setup_storage_manager().await;
    let test_key = SecretKey::random();
    let test_addr = ScratchpadAddress::new(test_key.public_key());
    let test_data = b"storage manager write-read cycle";

    let write_result = storage_manager
        .write_pad_data(&test_key, test_data, &PadStatus::Generated)
        .await;
    assert!(
        write_result.is_ok(),
        "write_pad_data failed: {:?}",
        write_result.err()
    );
    assert_eq!(
        write_result.unwrap(),
        test_addr,
        "write_pad_data address mismatch"
    );

    let read_result = storage_manager.read_pad_scratchpad(&test_addr).await;
    assert!(
        read_result.is_ok(),
        "read_pad_scratchpad failed: {:?}",
        read_result.err()
    );
}

#[tokio::test]
async fn test_write_pad_create_fails_if_exists() {
    let storage_manager = setup_storage_manager().await;
    let test_key = SecretKey::random();
    let test_data = b"create should fail second time";

    storage_manager
        .write_pad_data(&test_key, test_data, &PadStatus::Generated)
        .await
        .expect("Initial write_pad_data failed");

    let second_write_result = storage_manager
        .write_pad_data(&test_key, b"different data", &PadStatus::Generated)
        .await;

    assert!(
        second_write_result.is_err(),
        "Second write_pad_data should fail"
    );
    match second_write_result.err().unwrap() {
        StorageError::Network(NetworkError::InconsistentState(_)) => {}
        e => panic!(
            "Expected StorageError::Network(InconsistentState), got {:?}",
            e
        ),
    }
}

#[tokio::test]
async fn test_write_pad_update_fails_if_not_exists() {
    let storage_manager = setup_storage_manager().await;
    let test_key = SecretKey::random();

    let result = storage_manager
        .write_pad_data(&test_key, b"update should fail", &PadStatus::Written)
        .await;

    assert!(
        result.is_err(),
        "write_pad_data with Written status should fail if pad doesn't exist"
    );
    match result.err().unwrap() {
        StorageError::Network(NetworkError::InconsistentState(_)) => {}
        e => panic!(
            "Expected StorageError::Network(InconsistentState), got {:?}",
            e
        ),
    }
}

#[tokio::test]
async fn test_read_pad_fails_if_not_exists() {
    let storage_manager = setup_storage_manager().await;
    let test_key = SecretKey::random();
    let test_addr = ScratchpadAddress::new(test_key.public_key());

    let result = storage_manager.read_pad_scratchpad(&test_addr).await;

    assert!(
        result.is_err(),
        "read_pad_scratchpad should fail for non-existent pad"
    );

    match result.err().unwrap() {
        StorageError::Network(NetworkError::InternalError(msg)) => {
            assert!(
                msg.contains("RecordNotFound"),
                "Expected InternalError message to contain 'RecordNotFound', but got: {}",
                msg
            );
        }

        StorageError::Network(NetworkError::InconsistentState(msg)) => {
            assert!(msg.contains("not found") || msg.contains("RecordNotFound"),
                    "Expected InconsistentState message to indicate 'not found' or 'RecordNotFound', but got: {}", msg);
        }
        e => panic!(
            "Expected StorageError::Network(InternalError containing 'RecordNotFound'), got {:?}",
            e
        ),
    }
}
