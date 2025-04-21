#![cfg(test)]

use crate::data::chunking::{chunk_data, reassemble_data};
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::ops::fetch_public::fetch_public_op;
use crate::data::ops::store_public::store_public_op;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::IndexEntry;
use crate::index::structure::PadInfo;
use crate::index::structure::PadStatus;
use crate::network::adapter::create_public_scratchpad;
use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use rand::RngCore;
use serial_test::serial;
use std::assert_eq;
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
    // Add checks for sk_bytes and counter in the first pad
    let first_pad = &info.pads[0];
    assert!(!first_pad.sk_bytes.is_empty(), "sk_bytes should be stored");
    assert_eq!(
        first_pad.last_known_counter, 0,
        "Initial counter should be 0"
    );
    assert!(first_pad.receipt.is_none(), "Receipt should be None");
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
    use crate::index::structure::PadInfo;
    use serial_test::serial; // Ensure PadInfo imported here too

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
        let name = "test_public_basic".to_string();
        let mut data = vec![0u8; 1024]; // 1KB data
        rand::thread_rng().fill_bytes(&mut data);

        let store_res = data_manager.store_public(name.clone(), &data, None).await;
        assert!(
            store_res.is_ok(),
            "store_public failed: {:?}",
            store_res.err()
        );
        let public_index_addr = store_res.unwrap();

        // Verify index entry
        let index_info_res = index_manager.get_public_upload_info(&name).await;
        assert!(index_info_res.is_ok());
        let index_info_opt = index_info_res.unwrap();
        assert!(index_info_opt.is_some());
        let index_info = index_info_opt.unwrap();

        assert_eq!(index_info.index_pad.address, public_index_addr);
        assert_eq!(index_info.size, data.len());
        // Check data pads
        assert!(!index_info.data_pads.is_empty());
        for pad_info in &index_info.data_pads {
            assert_eq!(pad_info.last_known_counter, 0);
            assert!(!pad_info.sk_bytes.is_empty());
            assert_eq!(pad_info.status, PadStatus::Written);
            assert!(pad_info.receipt.is_some()); // Check receipt is Some
        }
        // Check index pad
        assert_eq!(index_info.index_pad.last_known_counter, 0);
        assert!(!index_info.index_pad.sk_bytes.is_empty());
        assert_eq!(index_info.index_pad.status, PadStatus::Written);
        assert!(index_info.index_pad.receipt.is_some()); // Check receipt is Some
    }

    #[tokio::test]
    #[serial]
    async fn test_store_public_op_empty_data() {
        let (_net_adapter, data_manager, index_manager) = setup_public_test_env().await;
        let name = "test_public_empty".to_string();
        let data = vec![];

        let store_res = data_manager.store_public(name.clone(), &data, None).await;
        assert!(
            store_res.is_ok(),
            "store_public (empty) failed: {:?}",
            store_res.err()
        );
        let public_index_addr = store_res.unwrap();

        // Verify index entry
        let index_info = index_manager
            .get_public_upload_info(&name)
            .await
            .unwrap()
            .expect("Index info not found for empty public store");

        assert_eq!(index_info.index_pad.address, public_index_addr);
        assert_eq!(index_info.size, 0);
        assert!(index_info.data_pads.is_empty());
        // Check index pad
        assert_eq!(index_info.index_pad.last_known_counter, 0);
        assert!(!index_info.index_pad.sk_bytes.is_empty());
        assert_eq!(index_info.index_pad.status, PadStatus::Written);
        assert!(index_info.index_pad.receipt.is_some()); // Check receipt is Some
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
        let (_net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "test_fetch_public_basic".to_string();
        let mut data = vec![0u8; 2048]; // 2KB data
        rand::thread_rng().fill_bytes(&mut data);

        let store_res = data_manager.store_public(name.clone(), &data, None).await;
        let public_index_addr = store_res.expect("store_public failed in fetch test");

        let fetch_res = data_manager.fetch_public(public_index_addr, None).await;
        assert!(
            fetch_res.is_ok(),
            "fetch_public failed: {:?}",
            fetch_res.err()
        );
        assert_eq!(fetch_res.unwrap().to_vec(), data);
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_empty() {
        let (_net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "test_fetch_public_empty".to_string();
        let data = vec![];

        let store_res = data_manager.store_public(name.clone(), &data, None).await;
        let public_index_addr = store_res.expect("store_public (empty) failed in fetch test");

        let fetch_res = data_manager.fetch_public(public_index_addr, None).await;
        assert!(
            fetch_res.is_ok(),
            "fetch_public (empty) failed: {:?}",
            fetch_res.err()
        );
        assert!(fetch_res.unwrap().is_empty());
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_public_op_not_found() {
        let (_net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let random_key = SecretKey::random();
        let non_existent_addr = ScratchpadAddress::new(random_key.public_key());
        let fetch_result = fetch_public_op(&data_manager, non_existent_addr, None).await;
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
        let (net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let sk = SecretKey::random();
        let invalid_encoding = 99u64;
        let addr = put_invalid_scratchpad(
            &net_adapter,
            &sk,
            invalid_encoding,
            b"wrong encoding index data",
        )
        .await;
        let fetch_result = fetch_public_op(&data_manager, addr, None).await;
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

    // --- Update Test ---
    #[tokio::test]
    #[serial]
    async fn test_update_public_op_basic_and_counter() {
        let (_net_adapter, data_manager, index_manager) = setup_public_test_env().await;
        let name = "test_public_update_basic".to_string();
        let mut data1 = vec![0u8; 1024]; // 1KB initial data
        rand::thread_rng().fill_bytes(&mut data1);
        let mut data2 = vec![0u8; 1024]; // 1KB updated data
        rand::thread_rng().fill_bytes(&mut data2);

        // 1. Initial Store
        let store_res = data_manager.store_public(name.clone(), &data1, None).await;
        assert!(store_res.is_ok(), "Initial store_public failed");
        let public_index_addr = store_res.unwrap();

        // Quick check initial state
        let initial_info = index_manager
            .get_public_upload_info(&name)
            .await
            .unwrap()
            .expect("Index info not found after initial store");
        assert_eq!(
            initial_info.data_pads[0].last_known_counter, 0,
            "Initial data pad counter mismatch"
        );
        assert_eq!(
            initial_info.index_pad.last_known_counter, 0,
            "Initial index pad counter mismatch"
        );

        // 2. Update
        let update_res = data_manager.update_public(&name, &data2, None).await;
        assert!(
            update_res.is_ok(),
            "update_public failed: {:?}",
            update_res.err()
        );

        // 3. Verify state after update
        let updated_info_res = index_manager.get_public_upload_info(&name).await;
        assert!(updated_info_res.is_ok());
        let updated_info = updated_info_res
            .unwrap()
            .expect("Index info not found after update");

        assert_eq!(
            updated_info.index_pad.address, public_index_addr,
            "Index address should not change"
        );
        assert_eq!(updated_info.size, data2.len());
        assert_eq!(
            updated_info.data_pads.len(),
            initial_info.data_pads.len(),
            "Data pad count should not change"
        );

        // Check counters are incremented
        assert_eq!(
            updated_info.data_pads[0].last_known_counter, 1,
            "Data pad counter should be incremented"
        );
        assert_eq!(
            updated_info.index_pad.last_known_counter, 1,
            "Index pad counter should be incremented"
        );
        // Check other fields remain valid
        assert!(!updated_info.data_pads[0].sk_bytes.is_empty());
        assert!(updated_info.data_pads[0].receipt.is_some()); // Expect receipt after update
        assert!(!updated_info.index_pad.sk_bytes.is_empty());
        assert!(updated_info.index_pad.receipt.is_some()); // Expect receipt after update

        // 4. Fetch updated data
        let fetch_res = data_manager.fetch_public(public_index_addr, None).await;
        assert!(
            fetch_res.is_ok(),
            "fetch_public after update failed: {:?}",
            fetch_res.err()
        );
        assert_eq!(
            fetch_res.unwrap().to_vec(),
            data2,
            "Fetched updated data mismatch"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_update_public_op_chunk_count_change_fails() {
        let (_net_adapter, data_manager, _index_manager) = setup_public_test_env().await;
        let name = "test_public_update_chunk_change".to_string();
        // Use the actual default scratchpad size
        let chunk_size = crate::index::structure::DEFAULT_SCRATCHPAD_SIZE;
        let data1 = vec![1u8; chunk_size]; // Exactly 1 chunk
        let data2 = vec![2u8; chunk_size + 1]; // Requires 2 chunks

        // Initial Store
        data_manager
            .store_public(name.clone(), &data1, None)
            .await
            .expect("Initial store failed");

        // Attempt Update with different chunk count
        let update_res = data_manager.update_public(&name, &data2, None).await;
        assert!(
            update_res.is_err(),
            "Update with chunk count change should fail"
        );
        match update_res.err().unwrap() {
            DataError::InvalidOperation(msg) => {
                assert!(
                    msg.contains("different number of chunks is not supported"),
                    "Incorrect error message: {}",
                    msg
                );
            }
            e => panic!("Expected InvalidOperation error, got {:?}", e),
        }
    }
}
