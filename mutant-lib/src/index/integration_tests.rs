#![cfg(test)]

use crate::index::manager::DefaultIndexManager;
use crate::index::structure::{KeyInfo, PadInfo, PadStatus};

use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::PadOrigin;
use crate::types::KeySummary;

use crate::data::PRIVATE_DATA_ENCODING;
use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn setup_test_components() -> (Arc<AutonomiNetworkAdapter>, DefaultIndexManager) {
    let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test adapter setup failed"),
    );
    let index_manager = DefaultIndexManager::new(Arc::clone(&network_adapter), SecretKey::random());
    (network_adapter, index_manager)
}

fn get_test_master_keys(test_id: &str) -> (SecretKey, ScratchpadAddress) {
    let base_key = SecretKey::random();

    let index_key = base_key.derive_child(format!("master_index_{}", test_id).as_bytes());
    let index_addr = ScratchpadAddress::new(index_key.public_key());
    (index_key, index_addr)
}

fn generate_random_pad(_id: &str) -> (ScratchpadAddress, SecretKey, Vec<u8>) {
    let key = SecretKey::random();
    let addr = ScratchpadAddress::new(key.public_key());
    let key_bytes = key.to_bytes().to_vec();
    (addr, key, key_bytes)
}

#[tokio::test]
async fn test_save_load_initialize() {
    let (network_adapter, index_manager) = setup_test_components();
    let (master_key, master_addr) = get_test_master_keys("save_load");

    index_manager
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Initial load failed");
    let initial_keys = index_manager
        .list_keys()
        .await
        .expect("Initial list_keys failed");
    assert!(initial_keys.is_empty(), "Index should be empty initially");

    let (pad1_addr, pad1_key, pad1_key_bytes) = generate_random_pad("pad1");

    let dummy_data = vec![0u8; 10];
    network_adapter
        .put_raw(
            &pad1_key,
            &dummy_data,
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await
        .expect("Failed to write dummy data to pad1 for test setup");

    let key_a_info = KeyInfo {
        data_size: 100,
        pads: vec![PadInfo {
            address: pad1_addr,
            chunk_index: 0,
            status: PadStatus::Written,
            origin: PadOrigin::Generated,
            needs_reverification: false,
        }],
        pad_keys: HashMap::from([(pad1_addr, pad1_key_bytes.clone())]),
        modified: Utc::now(),
        is_complete: false,
    };
    index_manager
        .insert_key_info("key_A".to_string(), key_a_info.clone())
        .await
        .expect("Insert key_A failed");

    index_manager
        .add_free_pad(pad1_addr, pad1_key_bytes)
        .await
        .expect("Add free pad failed");

    index_manager
        .save(&master_addr, &master_key)
        .await
        .expect("Save failed");

    let (_net_adapter2, index_manager2) = setup_test_components();
    index_manager2
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Second load failed");

    let loaded_keys = index_manager2
        .list_keys()
        .await
        .expect("Second list_keys failed");
    // Create the expected KeySummary for comparison
    let expected_summary = KeySummary {
        name: "key_A".to_string(),
        is_public: false,
        address: None,
    };
    assert_eq!(
        loaded_keys,
        vec![expected_summary], // Compare against Vec<KeySummary>
        "Loaded keys mismatch"
    );

    let loaded_key_a_info = index_manager2
        .get_key_info("key_A")
        .await
        .expect("Get key_A info failed")
        .expect("Key_A info not found after load");

    assert_eq!(
        loaded_key_a_info.data_size, key_a_info.data_size,
        "Loaded key size mismatch"
    );
    assert_eq!(
        loaded_key_a_info.pads.len(),
        1,
        "Loaded key pad count mismatch"
    );
    assert_eq!(
        loaded_key_a_info.pads[0].address, pad1_addr,
        "Loaded pad address mismatch"
    );

    let stats = index_manager2
        .get_storage_stats()
        .await
        .expect("Get stats failed");
    assert_eq!(stats.occupied_pads, 0, "Occupied pads should be 0");
    assert_eq!(stats.free_pads, 1, "Loaded free pad count mismatch");
}

#[tokio::test]
async fn test_load_non_existent_initializes_default() {
    let (_net_adapter, index_manager) = setup_test_components();

    let master_key = SecretKey::random();
    let master_addr = ScratchpadAddress::new(master_key.public_key());

    index_manager
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Load non-existent failed");

    let keys = index_manager.list_keys().await.expect("list_keys failed");
    assert!(
        keys.is_empty(),
        "Index should be empty after loading non-existent"
    );
    let stats = index_manager
        .get_storage_stats()
        .await
        .expect("get_stats failed");
    assert_eq!(stats.occupied_pads, 0, "Occupied pads should be 0");
    assert_eq!(stats.free_pads, 0, "Free pads should be 0");
}

#[tokio::test]
async fn test_reset() {
    let (_net_adapter, index_manager) = setup_test_components();
    let (master_key, master_addr) = get_test_master_keys("reset");

    let (pad1_addr, _, pad1_key_bytes) = generate_random_pad("pad_reset");
    index_manager
        .load_or_initialize(&master_addr, &master_key)
        .await
        .unwrap();
    let default_key_info = KeyInfo {
        pads: vec![],
        pad_keys: HashMap::new(),
        data_size: 0,
        modified: Utc::now(),
        is_complete: false,
    };
    index_manager
        .insert_key_info("key_to_reset".to_string(), default_key_info)
        .await
        .unwrap();
    index_manager
        .add_free_pad(pad1_addr, pad1_key_bytes)
        .await
        .unwrap();
    index_manager
        .save(&master_addr, &master_key)
        .await
        .expect("Initial save failed");

    index_manager
        .reset(&master_addr, &master_key)
        .await
        .expect("Reset failed");

    let keys = index_manager
        .list_keys()
        .await
        .expect("list_keys after reset failed");
    assert!(keys.is_empty(), "Index should be empty after reset");
    let stats = index_manager
        .get_storage_stats()
        .await
        .expect("get_stats after reset failed");
    assert_eq!(stats.occupied_pads, 0);
    assert_eq!(stats.free_pads, 0);

    let (_net_adapter2, index_manager2) = setup_test_components();
    index_manager2
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Load after reset failed");

    let loaded_keys = index_manager2
        .list_keys()
        .await
        .expect("list_keys after load failed");
    assert!(
        loaded_keys.is_empty(),
        "Loaded index should be empty after reset"
    );
    let loaded_stats = index_manager2
        .get_storage_stats()
        .await
        .expect("get_stats after load failed");
    assert_eq!(loaded_stats.occupied_pads, 0);
    assert_eq!(loaded_stats.free_pads, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::error::IndexError;
    use crate::index::manager::DefaultIndexManager;

    use crate::index::structure::{IndexEntry, KeyInfo, PublicUploadInfo};
    use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
    use autonomi::{ScratchpadAddress, SecretKey};
    use serial_test::serial;
    use std::collections::HashMap;

    // Helper to set up adapter and index manager for tests
    async fn setup_test_index_manager() -> (DefaultIndexManager, ScratchpadAddress, SecretKey) {
        let network_adapter = Arc::new(
            AutonomiNetworkAdapter::new(
                // Use a deterministic key for testing if needed, or random
                "0x112233445566778899aabbccddeeff00112233445566778899aabbccddeeff00",
                NetworkChoice::Devnet, // Use Devnet for consistency
            )
            .expect("Failed to create test network adapter"),
        );
        let master_key = SecretKey::random();
        let master_address = ScratchpadAddress::new(master_key.public_key());
        let index_manager = DefaultIndexManager::new(network_adapter, master_key.clone());
        // Ensure it starts empty
        index_manager
            .load_or_initialize(&master_address, &master_key)
            .await
            .expect("Failed to initialize index");
        (index_manager, master_address, master_key)
    }

    #[tokio::test]
    #[serial]
    async fn test_insert_public_upload_info() {
        let (index_manager, master_addr, master_key) = setup_test_index_manager().await;
        index_manager
            .load_or_initialize(&master_addr, &master_key)
            .await
            .unwrap();

        let name1 = "public_upload_1".to_string();
        let addr1 = ScratchpadAddress::new(SecretKey::random().public_key());
        let metadata1 = PublicUploadInfo {
            address: addr1,
            size: 1024,
            modified: Utc::now(),
            index_secret_key_bytes: SecretKey::random().to_bytes().to_vec(),
            data_pad_addresses: Vec::new(),
        };

        index_manager
            .insert_public_upload_info(name1.clone(), metadata1.clone())
            .await
            .unwrap();

        let index_copy1 = index_manager.get_index_copy().await.unwrap();
        assert_eq!(index_copy1.index.len(), 1);
        match index_copy1.index.get(&name1) {
            Some(IndexEntry::PublicUpload(info)) => assert_eq!(info, &metadata1),
            _ => panic!("Expected PublicUpload entry"),
        }

        let name2 = "public_upload_2".to_string();
        let addr2 = ScratchpadAddress::new(SecretKey::random().public_key());
        let metadata2 = PublicUploadInfo {
            address: addr2,
            size: 2048,
            modified: Utc::now(),
            index_secret_key_bytes: SecretKey::random().to_bytes().to_vec(),
            data_pad_addresses: Vec::new(),
        };

        index_manager
            .insert_public_upload_info(name2.clone(), metadata2.clone())
            .await
            .unwrap();

        let index_copy2 = index_manager.get_index_copy().await.unwrap();
        assert_eq!(index_copy2.index.len(), 2);
        match index_copy2.index.get(&name2) {
            Some(IndexEntry::PublicUpload(info)) => assert_eq!(info, &metadata2),
            _ => panic!("Expected PublicUpload entry"),
        }

        // Test inserting the same name again (should fail)
        let result = index_manager
            .insert_public_upload_info(name1.clone(), metadata1.clone())
            .await;
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), IndexError::KeyExists(_)));

        // Insert a private key with a different name
        let name3 = "private_key_3".to_string();
        let metadata3 = KeyInfo {
            pads: vec![],
            pad_keys: HashMap::new(),
            data_size: 500,
            modified: Utc::now(),
            is_complete: true,
        };
        index_manager
            .insert_key_info(name3.clone(), metadata3.clone())
            .await
            .unwrap();

        let index_copy3 = index_manager.get_index_copy().await.unwrap();
        assert_eq!(index_copy3.index.len(), 3);
        match index_copy3.index.get(&name1) {
            Some(IndexEntry::PublicUpload(info)) => assert_eq!(info, &metadata1),
            _ => panic!("Expected PublicUpload entry for name1"),
        }
        match index_copy3.index.get(&name2) {
            Some(IndexEntry::PublicUpload(info)) => assert_eq!(info, &metadata2),
            _ => panic!("Expected PublicUpload entry for name2"),
        }
        match index_copy3.index.get(&name3) {
            Some(IndexEntry::PrivateKey(info)) => assert_eq!(info, &metadata3),
            _ => panic!("Expected PrivateKey entry for name3"),
        }

        // Test inserting a public upload with the same name as a private key (should fail)
        let result_collide = index_manager
            .insert_public_upload_info(name3.clone(), metadata1.clone())
            .await;
        assert!(result_collide.is_err());
        assert!(matches!(
            result_collide.err().unwrap(),
            IndexError::KeyExists(_)
        ));
    }
}
