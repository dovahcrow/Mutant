#![cfg(test)]

use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::ops::fetch_public::fetch_public_op;
use crate::data::ops::store_public::store_public_op;
use crate::index::error::IndexError;
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::IndexEntry;
use crate::network::adapter::create_public_scratchpad;
use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use serial_test::serial;
use std::sync::Arc;

// Define constant locally
const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn setup_managers() -> (
    Arc<AutonomiNetworkAdapter>,
    Arc<DefaultIndexManager>,
    Arc<DefaultPadLifecycleManager>,
    DefaultDataManager,
) {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await; // Prevent rate limiting

    let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test NetworkAdapter setup failed"),
    );

    let master_key_for_index = SecretKey::random();
    let index_manager = Arc::new(DefaultIndexManager::new(
        network_adapter.clone(),
        master_key_for_index,
    ));
    // Use load_or_initialize, assuming a dummy key/address is fine for data tests
    // If these tests require specific index state, this setup needs adjustment.
    let dummy_key = autonomi::SecretKey::random();
    let dummy_addr = autonomi::ScratchpadAddress::new(dummy_key.public_key());
    index_manager
        .load_or_initialize(&dummy_addr, &dummy_key)
        .await
        .expect("Index initialization failed in data test setup");
    // Save might not be strictly needed here, but keeping for consistency
    index_manager
        .save(&dummy_addr, &dummy_key)
        .await
        .expect("Initial index save failed in data test setup");

    let pad_lifecycle_manager = Arc::new(DefaultPadLifecycleManager::new(
        index_manager.clone(),
        network_adapter.clone(),
    ));

    let data_manager = DefaultDataManager::new(
        network_adapter.clone(),
        index_manager.clone(),
        pad_lifecycle_manager.clone(),
    );

    // Need to pass TEST_SCRATCHPAD_SIZE to store/fetch calls now, not constructor

    (
        network_adapter,
        index_manager,
        pad_lifecycle_manager,
        data_manager,
    )
}

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
    assert_eq!(chunks.len(), 0);
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
    let expected_size = 4;
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
    let expected_size = 3;
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

#[tokio::test]
async fn test_store_very_large_data_multi_pad() {
    let (_network_adapter, index_manager, _pad_lifecycle_manager, data_manager) =
        setup_managers().await;

    let key = "very_large_data_key".to_string();

    let data_size = 16 * 1024 * 1024;
    let data = vec![1u8; data_size];

    let store_result = data_manager.store(key.clone(), &data, None).await;
    assert!(
        store_result.is_ok(),
        "Store very large data failed: {:?}",
        store_result.err()
    );

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
    let expected_pads = 5;
    assert_eq!(
        key_info.pads.len(),
        expected_pads,
        "Expected exactly {} pads for 16MiB data, found {}",
        expected_pads,
        key_info.pads.len()
    );
    assert_eq!(key_info.data_size, data.len());

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

    let remove_result = data_manager.remove(&key).await;
    assert!(
        remove_result.is_ok(),
        "Remove very large data failed: {:?}",
        remove_result.err()
    );

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

#[tokio::test]
async fn test_store_basic() {
    let (_network, index_manager, _pad_lifecycle, data_manager) = setup_managers().await;
    let key = "store_basic_key".to_string();
    let data = b"some simple data".to_vec();

    let store_result = data_manager.store(key.clone(), &data, None).await;
    assert!(
        store_result.is_ok(),
        "Store failed: {:?}",
        store_result.err()
    );

    // Verification: Check index
    let key_info_res = index_manager.get_key_info(&key).await;
    assert!(key_info_res.is_ok());
    let key_info = key_info_res.unwrap();
    assert!(key_info.is_some());
    let info = key_info.unwrap();
    assert_eq!(info.data_size as usize, data.len());
    assert!(!info.pads.is_empty());
}

#[tokio::test]
async fn test_fetch_basic() {
    let (_network, _index_manager, _pad_lifecycle, data_manager) = setup_managers().await;
    let key = "fetch_basic_key".to_string();
    let data = b"some data to fetch".to_vec();

    data_manager.store(key.clone(), &data, None).await.unwrap();

    let fetch_result = data_manager.fetch(&key, None).await;
    assert!(
        fetch_result.is_ok(),
        "Fetch failed: {:?}",
        fetch_result.err()
    );
    assert_eq!(fetch_result.unwrap(), data);
}

#[tokio::test]
async fn test_store_fetch_remove_cycle() {
    let (_network_adapter, index_manager, _pad_lifecycle_manager, data_manager) =
        setup_managers().await;
    let key = "store_fetch_remove_cycle_key".to_string();
    let data = b"data for full cycle test".to_vec();

    // Store
    assert!(data_manager.store(key.clone(), &data, None).await.is_ok());

    // Fetch
    let fetched_data = data_manager.fetch(&key, None).await;
    assert!(fetched_data.is_ok());
    assert_eq!(fetched_data.unwrap(), data);

    // Remove
    assert!(data_manager.remove(&key).await.is_ok());

    // Fetch again (should fail)
    let fetch_after_remove = data_manager.fetch(&key, None).await;
    assert!(fetch_after_remove.is_err());
    match fetch_after_remove.err().unwrap() {
        DataError::KeyNotFound(_) => {} // Expected
        e => panic!("Expected KeyNotFound after remove, got {:?}", e),
    }

    // Check index manager state (key should be gone)
    let key_info_after_remove = index_manager.get_key_info(&key).await;
    assert!(key_info_after_remove.is_ok());
    assert!(key_info_after_remove.unwrap().is_none());
}

#[tokio::test]
async fn test_store_existing_key() {
    let (_network_adapter, _index_manager, _pad_lifecycle_manager, data_manager) =
        setup_managers().await;
    let key = "existing_key_test".to_string();
    let data1 = b"initial data".to_vec();
    let data2 = b"new data".to_vec();

    // Store initial data
    assert!(data_manager.store(key.clone(), &data1, None).await.is_ok());

    // Try to store again (should fail by default)
    let store_again = data_manager.store(key.clone(), &data2, None).await;
    assert!(store_again.is_err());
    match store_again.err().unwrap() {
        DataError::KeyAlreadyExists(_) => {} // Expected
        e => panic!("Expected KeyAlreadyExists, got {:?}", e),
    }

    // TODO: Add test for --force overwrite once implemented
}

#[tokio::test]
async fn test_fetch_nonexistent_key() {
    let (_network_adapter, _index_manager, _pad_lifecycle_manager, data_manager) =
        setup_managers().await;
    let key = "nonexistent_key_fetch".to_string();

    let result = data_manager.fetch(&key, None).await;
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::KeyNotFound(_) => {} // Expected
        e => panic!("Expected KeyNotFound, got {:?}", e),
    }
}

#[tokio::test]
async fn test_remove_nonexistent_key() {
    let (_network_adapter, _index_manager, _pad_lifecycle_manager, data_manager) =
        setup_managers().await;
    let key = "nonexistent_key_remove".to_string();

    let result = data_manager.remove(&key).await;
    assert!(result.is_err());
    match result.err().unwrap() {
        DataError::KeyNotFound(_) => {} // Expected
        e => panic!("Expected KeyNotFound, got {:?}", e),
    }
}

// === Public Ops Tests ===

#[cfg(test)]
mod public_op_tests {
    use super::*; // Bring outer scope items in
    use crate::data::error::DataError; // Specific errors might be needed
    use crate::index::error::IndexError;
    use serial_test::serial;

    // Renamed helper to avoid potential conflicts
    async fn setup_public_test_env() -> (
        Arc<AutonomiNetworkAdapter>,
        DefaultDataManager,
        Arc<DefaultIndexManager>,
    ) {
        // Use setup_managers from outer scope?
        // Re-implement slightly to return needed types directly
        let network_adapter = Arc::new(
            AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
                .expect("Failed to create test network adapter"),
        );
        let master_key = SecretKey::random();
        let master_address = ScratchpadAddress::new(master_key.public_key());
        let index_manager = Arc::new(DefaultIndexManager::new(
            network_adapter.clone(),
            master_key.clone(),
        ));
        index_manager
            .load_or_initialize(&master_address, &master_key)
            .await
            .expect("Failed to initialize index");
        let pad_lifecycle_manager = Arc::new(
            crate::pad_lifecycle::manager::DefaultPadLifecycleManager::new(
                index_manager.clone(),
                network_adapter.clone(),
            ),
        );
        let data_manager = DefaultDataManager::new(
            network_adapter.clone(),
            index_manager.clone(),
            pad_lifecycle_manager,
        );
        (network_adapter, data_manager, index_manager)
    }

    // --- Store Tests ---
    #[tokio::test]
    #[serial]
    async fn test_store_public_op_basic() {
        let (net_adapter, data_manager, index_manager) = setup_public_test_env().await;
        let name = "public_store_basic".to_string();
        let data = b"some public data here";
        let result = store_public_op(&data_manager, name.clone(), data, None).await;
        assert!(result.is_ok(), "store_public_op failed: {:?}", result.err());
        let public_index_addr = result.unwrap();
        let index_copy = index_manager.get_index_copy().await.unwrap();
        let metadata = index_copy.public_uploads.get(&name);
        assert!(metadata.is_some(), "Metadata not found in index");
        assert_eq!(
            metadata.unwrap().address,
            public_index_addr,
            "Stored address mismatch"
        );

        // Check index state
        let index_copy = index_manager.get_index_copy().await.unwrap();
        assert!(index_copy.index.contains_key(&name));
        let entry = index_copy.index.get(&name).unwrap();
        let metadata = match entry {
            IndexEntry::PublicUpload(info) => info,
            _ => panic!("Expected PublicUpload entry, found PrivateKey"),
        };
        assert_eq!(metadata.size, data.len());

        // Clean up (remove is not implemented for public, just clear index state)
        // data_manager.remove(&name).await.unwrap(); // Assume remove works for public names too?
        // For now, just manually remove from index for test isolation
        index_manager.remove_key_info(&name).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_store_public_op_empty_data() {
        let (net_adapter, data_manager, index_manager) = setup_public_test_env().await;
        let name = "public_empty_data".to_string();
        let data: Vec<u8> = vec![];

        let result = store_public_op(&data_manager, name.clone(), &data, None).await;
        assert!(result.is_ok());
        let public_addr = result.unwrap();

        // Check index state
        let index_copy = index_manager.get_index_copy().await.unwrap();
        let entry = index_copy.index.get(&name).unwrap();
        let metadata = match entry {
            IndexEntry::PublicUpload(info) => info,
            _ => panic!("Expected PublicUpload entry, found PrivateKey"),
        };
        assert_eq!(metadata.size, 0);
        assert_eq!(metadata.address, public_addr);

        // Fetch the empty data via public fetch op
        let fetched_data = fetch_public_op(&net_adapter, public_addr, None).await;
        assert!(fetched_data.is_ok());
        assert!(fetched_data.unwrap().is_empty());

        // Clean up
        index_manager.remove_key_info(&name).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_store_public_op_name_collision() {
        let (_net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "public_store_collision".to_string();
        let data1 = b"data one";
        let data2 = b"data two";
        let result1 = store_public_op(&data_manager, name.clone(), data1, None).await;
        assert!(result1.is_ok(), "First store failed: {:?}", result1.err());
        let result2 = store_public_op(&data_manager, name.clone(), data2, None).await;
        assert!(result2.is_err(), "Second store should have failed");
        match result2.err().unwrap() {
            DataError::Index(IndexError::PublicUploadNameExists(collided_name)) => {
                assert_eq!(collided_name, name, "Incorrect collision name reported");
            }
            e => panic!("Expected PublicUploadNameExists error, got {:?}", e),
        }
    }

    // --- Fetch Tests ---
    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_basic() {
        let (net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "public_fetch_basic_integrated".to_string();
        let data = b"some fetchable public data integrated";
        let store_result = store_public_op(&data_manager, name.clone(), data, None).await;
        assert!(store_result.is_ok(), "Setup store failed");
        let public_index_addr = store_result.unwrap();

        let fetch_result = fetch_public_op(&net_adapter, public_index_addr, None).await;
        assert!(
            fetch_result.is_ok(),
            "fetch_public_op failed: {:?}",
            fetch_result.err()
        );
        let fetched_data = fetch_result.unwrap();
        assert_eq!(fetched_data.as_ref(), data, "Fetched data mismatch");
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_empty() {
        let (net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "public_fetch_empty_integrated".to_string();
        let data = b"";
        let store_result = store_public_op(&data_manager, name.clone(), data, None).await;
        assert!(store_result.is_ok(), "Setup store for empty failed");
        let public_index_addr = store_result.unwrap();
        let fetch_result = fetch_public_op(&net_adapter, public_index_addr, None).await;
        assert!(
            fetch_result.is_ok(),
            "fetch_public_op for empty failed: {:?}",
            fetch_result.err()
        );
        let fetched_data = fetch_result.unwrap();
        assert!(fetched_data.is_empty(), "Fetched data should be empty");
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_not_found() {
        let (net_adapter, _data_manager, _index_manager) = setup_public_test_env().await;
        let random_key = SecretKey::random();
        let non_existent_addr = ScratchpadAddress::new(random_key.public_key());
        let fetch_result = fetch_public_op(&net_adapter, non_existent_addr, None).await;
        assert!(
            fetch_result.is_err(),
            "Fetch should fail for non-existent addr"
        );
        match fetch_result.err().unwrap() {
            DataError::PublicScratchpadNotFound(addr) => {
                assert_eq!(
                    addr, non_existent_addr,
                    "Incorrect address in NotFound error"
                );
            }
            e => panic!("Expected PublicScratchpadNotFound error, got {:?}", e),
        }
    }

    // Helper to manually put an invalid scratchpad
    async fn put_invalid_scratchpad(
        net_adapter: &Arc<AutonomiNetworkAdapter>,
        sk: &SecretKey,
        encoding: u64,
        data: &[u8],
    ) -> ScratchpadAddress {
        // Fix lifetime issue: use copy_from_slice
        let scratchpad = create_public_scratchpad(sk, encoding, &Bytes::copy_from_slice(data), 0);
        let addr = *scratchpad.address();
        let payment = PaymentOption::Wallet((*net_adapter.wallet()).clone());
        net_adapter
            .scratchpad_put(scratchpad, payment)
            .await
            .expect("Manual put failed");
        addr
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_wrong_index_encoding() {
        let (net_adapter, _data_manager, _index_manager) = setup_public_test_env().await;
        let sk = SecretKey::random();
        let invalid_encoding = 99u64;
        let addr = put_invalid_scratchpad(
            &net_adapter,
            &sk,
            invalid_encoding,
            b"wrong encoding index data",
        )
        .await;
        let fetch_result = fetch_public_op(&net_adapter, addr, None).await;
        assert!(
            fetch_result.is_err(),
            "Fetch should fail for wrong index encoding"
        );
        match fetch_result.err().unwrap() {
            DataError::InvalidPublicIndexEncoding(enc) => {
                assert_eq!(enc, invalid_encoding, "Incorrect encoding reported");
            }
            e => panic!("Expected InvalidPublicIndexEncoding error, got {:?}", e),
        }
    }
}
