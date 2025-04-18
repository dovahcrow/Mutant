#![cfg(test)]

use crate::index::manager::{DefaultIndexManager, IndexManager};
use crate::index::structure::{KeyInfo, PadInfo, PadStatus};

use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::PadOrigin;
use crate::storage::manager::DefaultStorageManager;

use autonomi::{ScratchpadAddress, SecretKey};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn setup_test_components() -> (
    Arc<AutonomiNetworkAdapter>,
    Arc<DefaultStorageManager>,
    DefaultIndexManager,
) {
    let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test adapter setup failed"),
    );
    let storage_manager: Arc<DefaultStorageManager> =
        Arc::new(DefaultStorageManager::new(Arc::clone(&network_adapter)));
    let index_manager =
        DefaultIndexManager::new(Arc::clone(&storage_manager), Arc::clone(&network_adapter));
    (network_adapter, storage_manager, index_manager)
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
    let (_net_adapter, storage_manager, index_manager) = setup_test_components();
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
    storage_manager
        .write_pad_data(&pad1_key, &dummy_data, &PadStatus::Generated)
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

    let (_net_adapter2, _storage_manager2, index_manager2) = setup_test_components();
    index_manager2
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Second load failed");

    let loaded_keys = index_manager2
        .list_keys()
        .await
        .expect("Second list_keys failed");
    assert_eq!(
        loaded_keys,
        vec!["key_A".to_string()],
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
    let (_net_adapter, _storage_manager, index_manager) = setup_test_components();

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
    let (_net_adapter, _storage_manager, index_manager) = setup_test_components();
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

    let (_net_adapter2, _storage_manager2, index_manager2) = setup_test_components();
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
