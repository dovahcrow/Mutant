#![cfg(test)]

use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::data::manager::{DataManager, DefaultDataManager};
use crate::index::manager::{DefaultIndexManager, IndexManager};
use crate::network::adapter::{AutonomiNetworkAdapter, NetworkAdapter};
use crate::network::NetworkChoice;
use crate::pad_lifecycle::manager::{DefaultPadLifecycleManager, PadLifecycleManager};
use crate::storage::manager::{DefaultStorageManager, StorageManager};
use autonomi::{ScratchpadAddress, SecretKey};
use std::sync::Arc;

// --- Test Setup Helper ---

// Use a known devnet key for setting up adapters/managers consistently
const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Creates the components needed for DataManager integration tests.
/// Initializes the IndexManager with a random master key.
async fn setup_data_test_components() -> (
    Arc<dyn NetworkAdapter>,
    Arc<dyn StorageManager>,
    Arc<dyn IndexManager>,
    Arc<dyn PadLifecycleManager>,
    DefaultDataManager,
    SecretKey, // Return master key for potential use in tests
    ScratchpadAddress,
) {
    let network_adapter: Arc<dyn NetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test NetworkAdapter setup failed"),
    );
    let storage_manager: Arc<dyn StorageManager> =
        Arc::new(DefaultStorageManager::new(network_adapter.clone()));
    let index_manager_impl = Arc::new(DefaultIndexManager::new(
        storage_manager.clone(),
        network_adapter.clone(),
    ));
    let index_manager: Arc<dyn IndexManager> = index_manager_impl.clone();

    // Generate random master key/address for index isolation
    let master_key = SecretKey::random();
    let master_addr = ScratchpadAddress::new(master_key.public_key());

    // Initialize the index manager (load or create default)
    index_manager_impl
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Index manager initialization failed");

    let pad_lifecycle_manager_impl = Arc::new(DefaultPadLifecycleManager::new(
        index_manager.clone(),
        network_adapter.clone(),
        storage_manager.clone(),
    ));
    let pad_lifecycle_manager: Arc<dyn PadLifecycleManager> = pad_lifecycle_manager_impl;

    let data_manager = DefaultDataManager::new(
        index_manager.clone(),
        pad_lifecycle_manager.clone(),
        storage_manager.clone(),
        network_adapter.clone(),
    );

    (
        network_adapter,
        storage_manager,
        index_manager,
        pad_lifecycle_manager,
        data_manager,
        master_key,
        master_addr,
    )
}

// --- Chunking Unit Tests ---

#[test]
fn test_chunking_basic() {
    let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let chunk_size = 3;
    let chunks = chunk_data(&data, chunk_size).unwrap();
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0], vec![1, 2, 3]);
    assert_eq!(chunks[1], vec![4, 5, 6]);
    assert_eq!(chunks[2], vec![7, 8, 9]);
    assert_eq!(chunks[3], vec![10]);
}

#[test]
fn test_chunking_exact_multiple() {
    let data = vec![1, 2, 3, 4, 5, 6];
    let chunk_size = 3;
    let chunks = chunk_data(&data, chunk_size).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0], vec![1, 2, 3]);
    assert_eq!(chunks[1], vec![4, 5, 6]);
}

#[test]
fn test_chunking_larger_than_data() {
    let data = vec![1, 2, 3];
    let chunk_size = 10;
    let chunks = chunk_data(&data, chunk_size).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], vec![1, 2, 3]);
}

#[test]
fn test_chunking_empty_data() {
    let data = vec![];
    let chunk_size = 10;
    let chunks = chunk_data(&data, chunk_size).unwrap();
    assert_eq!(chunks.len(), 0); // Should be 0 chunks for empty data
}

#[test]
fn test_chunking_zero_size() {
    let data = vec![1, 2, 3];
    let chunk_size = 0;
    let result = chunk_data(&data, chunk_size);
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::ChunkingError(msg) => assert_eq!(msg, "Chunk size cannot be zero"),
        _ => panic!("Unexpected error type"),
    }
}

#[test]
fn test_reassemble_basic() {
    let chunks = vec![
        Some(vec![1, 2, 3]),
        Some(vec![4, 5, 6]),
        Some(vec![7, 8, 9]),
        Some(vec![10]),
    ];
    let expected_size = 10;
    let data = reassemble_data(chunks, expected_size).unwrap();
    assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn test_reassemble_empty() {
    let chunks = vec![];
    let expected_size = 0;
    let data = reassemble_data(chunks, expected_size).unwrap();
    assert_eq!(data, Vec::<u8>::new());
}

#[test]
fn test_reassemble_size_mismatch_too_small() {
    let chunks = vec![Some(vec![1, 2]), Some(vec![3])];
    let expected_size = 4; // Expecting more data
    let result = reassemble_data(chunks, expected_size);
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::ReassemblyError(msg) => {
            assert!(msg.contains("does not match expected size"))
        }
        _ => panic!("Unexpected error type"),
    }
}

#[test]
fn test_reassemble_size_mismatch_too_large() {
    let chunks = vec![Some(vec![1, 2]), Some(vec![3, 4, 5])];
    let expected_size = 3; // Expecting less data
    let result = reassemble_data(chunks, expected_size);
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::ReassemblyError(msg) => {
            assert!(msg.contains("does not match expected size"))
        }
        _ => panic!("Unexpected error type"),
    }
}

#[test]
fn test_reassemble_missing_chunk() {
    let chunks = vec![Some(vec![1, 2]), None, Some(vec![5, 6])];
    let expected_size = 6;
    let result = reassemble_data(chunks, expected_size);
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::ReassemblyError(msg) => assert!(msg.contains("Missing chunk at index 1")),
        _ => panic!("Unexpected error type"),
    }
}

// --- Integration Tests for DataManager ---

#[tokio::test]
async fn test_store_very_large_data_multi_pad() {
    let (
        _network_adapter,
        _storage_manager,
        index_manager,
        _pad_lifecycle_manager,
        data_manager,
        _master_key,
        _master_addr,
    ) = setup_data_test_components().await;

    let key = "very_large_data_key".to_string();
    // Create data larger than 3 pads (16 MiB / ~4 MiB per pad = 4 pads)
    let data_size = 16 * 1024 * 1024;
    let data = vec![1u8; data_size]; // Use a non-zero byte for content

    // Store
    let store_result = data_manager.store(key.clone(), &data, None).await;
    assert!(
        store_result.is_ok(),
        "Store very large data failed: {:?}",
        store_result.err()
    );

    // Verify index has exactly 4 pads
    let key_info_res = index_manager.get_key_info(&key).await;
    assert!(
        key_info_res.is_ok(),
        "get_key_info failed: {:?}",
        key_info_res.err()
    );
    let key_info_opt = key_info_res.unwrap();
    assert!(
        key_info_opt.is_some(),
        "Key info not found after store (very large)"
    );
    let key_info = key_info_opt.unwrap();
    let expected_pads = 5; // 16MiB / (~4MiB - 512 bytes) requires 5 pads
    assert_eq!(
        key_info.pads.len(),
        expected_pads,
        "Expected exactly {} pads for 16MiB data, found {}",
        expected_pads,
        key_info.pads.len()
    );
    assert_eq!(key_info.data_size, data.len());

    // Fetch
    let fetch_result = data_manager.fetch(&key, None).await;
    assert!(
        fetch_result.is_ok(),
        "Fetch very large data failed: {:?}",
        fetch_result.err()
    );
    assert_eq!(
        fetch_result.unwrap(),
        data,
        "Fetched very large data mismatch"
    );

    // Remove
    let remove_result = data_manager.remove(&key).await;
    assert!(
        remove_result.is_ok(),
        "Remove very large data failed: {:?}",
        remove_result.err()
    );

    // Verify removed from index
    let key_info_after_remove_res = index_manager.get_key_info(&key).await;
    assert!(
        key_info_after_remove_res.is_ok(),
        "get_key_info after remove failed: {:?}",
        key_info_after_remove_res.err()
    );
    let key_info_after_remove_opt = key_info_after_remove_res.unwrap();
    assert!(
        key_info_after_remove_opt.is_none(),
        "Key info still present after remove (very large)"
    );
}
