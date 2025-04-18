#![cfg(test)]

use crate::index::manager::DefaultIndexManager;
use crate::index::structure::PadStatus;
use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::manager::DefaultPadLifecycleManager;
use crate::pad_lifecycle::PadOrigin;
use autonomi::{ScratchpadAddress, SecretKey};
use std::sync::Arc;
use tokio::runtime::Runtime;

// Re-define constant locally as test_utils module doesn't seem to exist at crate root
const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn setup_test_components_with_initialized_index() -> (
    Arc<AutonomiNetworkAdapter>,
    Arc<DefaultIndexManager>,
    DefaultPadLifecycleManager,
    SecretKey,
    ScratchpadAddress,
) {
    let network_adapter: Arc<AutonomiNetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test NetworkAdapter setup failed"),
    );
    let master_key_for_index = SecretKey::random();
    let index_manager = Arc::new(DefaultIndexManager::new(
        Arc::clone(&network_adapter),
        master_key_for_index.clone(),
    ));

    let master_key = SecretKey::random();
    let master_addr = ScratchpadAddress::new(master_key.public_key());

    index_manager
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Index manager initialization failed");

    let pad_lifecycle_manager =
        DefaultPadLifecycleManager::new(index_manager.clone(), network_adapter.clone());

    (
        network_adapter,
        index_manager,
        pad_lifecycle_manager,
        master_key,
        master_addr,
    )
}

#[test]
fn test_acquire_new_pads() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (_network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
            setup_test_components_with_initialized_index().await;

        let mut callback = None;
        let count = 1;

        let result = pad_lifecycle_manager
            .acquire_pads(count, &mut callback)
            .await;

        assert!(result.is_ok(), "acquire_pads failed: {:?}", result.err());
        let acquired_pads = result.unwrap();
        assert_eq!(
            acquired_pads.len(),
            count,
            "Incorrect number of pads acquired"
        );

        let (_pad_address, _pad_key, origin) = &acquired_pads[0];
        assert_eq!(
            *origin,
            PadOrigin::Generated,
            "Acquired pad should have Generated origin"
        );

        let index_state_after = index_manager.get_index_copy().await.unwrap();
        assert!(
            index_state_after.index.is_empty(),
            "Index map should remain empty after acquire_pads"
        );
        assert!(
            index_state_after.free_pads.is_empty(),
            "Free pool should remain empty"
        );
    });
}

#[tokio::test]
async fn test_acquire_pads_from_free_pool() {
    let (network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let num_pads_to_add = 3;
    let mut free_pads_info = Vec::new();
    for i in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        network_adapter
            .put_raw(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .unwrap_or_else(|e| panic!("Failed to write dummy data for pad {}: {}", i, e));

        free_pads_info.push((address, key_bytes));
    }

    index_manager
        .add_free_pads(free_pads_info.clone())
        .await
        .expect("Failed to add pads to free list");

    let num_pads_to_acquire = 2;
    let mut callback = None;
    let result = pad_lifecycle_manager
        .acquire_pads(num_pads_to_acquire, &mut callback)
        .await;

    assert!(result.is_ok(), "acquire_pads failed: {:?}", result.err());
    let acquired_pads = result.unwrap();
    assert_eq!(
        acquired_pads.len(),
        num_pads_to_acquire,
        "Incorrect number of pads acquired"
    );

    let mut acquired_addresses = std::collections::HashSet::new();
    for (address, _key, origin) in &acquired_pads {
        assert_eq!(
            *origin,
            PadOrigin::FreePool { initial_counter: 0 },
            "Acquired pad should have FreePool origin with counter 0"
        );
        acquired_addresses.insert(*address);
    }

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_after.free_pads.len(),
        num_pads_to_add - num_pads_to_acquire,
        "Incorrect number of pads remaining in free pool"
    );

    assert!(!index_state_after.free_pads.is_empty());
    let remaining_free_addr = index_state_after.free_pads[0].0;
    assert!(!acquired_addresses.contains(&remaining_free_addr));

    assert!(
        index_state_after.index.is_empty(),
        "Index map should remain empty"
    );
}

#[tokio::test]
async fn test_acquire_mixed_pads() {
    let (network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let num_free_pads_to_add = 2;
    let mut free_pads_info = Vec::new();
    for i in 0..num_free_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        network_adapter
            .put_raw(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .unwrap_or_else(|e| panic!("Failed to write dummy data for pad {}: {}", i, e));
        free_pads_info.push((address, key_bytes));
    }
    index_manager
        .add_free_pads(free_pads_info.clone())
        .await
        .expect("Failed to add pads to free list");

    let num_pads_to_acquire = 3;
    let mut callback = None;
    let result = pad_lifecycle_manager
        .acquire_pads(num_pads_to_acquire, &mut callback)
        .await;

    assert!(result.is_ok(), "acquire_pads failed: {:?}", result.err());
    let acquired_pads = result.unwrap();
    assert_eq!(
        acquired_pads.len(),
        num_pads_to_acquire,
        "Incorrect number of pads acquired"
    );

    let mut free_pool_count = 0;
    let mut generated_count = 0;
    for (_, _, origin) in &acquired_pads {
        match origin {
            PadOrigin::FreePool { initial_counter } => {
                assert_eq!(*initial_counter, 0, "FreePool pad should have counter 0");
                free_pool_count += 1;
            }
            PadOrigin::Generated => {
                generated_count += 1;
            }
        }
    }
    assert_eq!(
        free_pool_count, num_free_pads_to_add,
        "Incorrect number of FreePool pads acquired"
    );
    assert_eq!(
        generated_count,
        num_pads_to_acquire - num_free_pads_to_add,
        "Incorrect number of Generated pads acquired"
    );

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_after.free_pads.is_empty(),
        "Free pool should be empty after acquiring all free pads"
    );
    assert!(
        index_state_after.index.is_empty(),
        "Index map should remain empty"
    );
}

#[tokio::test]
async fn test_purge_existing_pads() {
    let (network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let num_pads_to_add = 2;
    let mut pending_pads_info = Vec::new();
    let mut expected_free_addresses = std::collections::HashSet::new();

    for _ in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        network_adapter
            .put_raw(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .expect("Failed to write test pad data for purge");

        pending_pads_info.push((address, key_bytes));
        expected_free_addresses.insert(address);
    }

    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_pads_to_add
    );
    assert!(index_state_before.free_pads.is_empty());

    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_after.pending_verification_pads.is_empty(),
        "Pending list should be empty after purge"
    );
    assert_eq!(
        index_state_after.free_pads.len(),
        num_pads_to_add,
        "Incorrect number of pads moved to free list"
    );

    let final_free_addresses: std::collections::HashSet<_> = index_state_after
        .free_pads
        .iter()
        .map(|(addr, _, _)| *addr)
        .collect();
    assert_eq!(
        final_free_addresses, expected_free_addresses,
        "Addresses in free list do not match expected addresses"
    );

    for (_, _, counter) in &index_state_after.free_pads {
        assert_eq!(*counter, 0, "Pad counter in free list should be 0");
    }
}

#[tokio::test]
async fn test_purge_non_existent_pads() {
    let (_network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let num_pads_to_add = 2;
    let mut pending_pads_info = Vec::new();

    for _i in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        pending_pads_info.push((address, key_bytes));
    }

    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_pads_to_add
    );
    assert!(index_state_before.free_pads.is_empty());

    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_after.pending_verification_pads.is_empty(),
        "Pending list should be empty after purge (non-existent pads removed)"
    );
    assert!(
        index_state_after.free_pads.is_empty(),
        "Free list should be empty (no pads verified)"
    );
}

#[tokio::test]
async fn test_purge_mixed_pads() {
    let (network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let num_existing = 2;
    let num_non_existent = 3;
    let mut pending_pads_info = Vec::new();
    let mut expected_free_addresses = std::collections::HashSet::new();

    for _ in 0..num_existing {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        network_adapter
            .put_raw(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .expect("Failed to write test pad data for purge");
        pending_pads_info.push((address, key_bytes));
        expected_free_addresses.insert(address);
    }

    for _i in 0..num_non_existent {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        pending_pads_info.push((address, key_bytes));
    }

    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_existing + num_non_existent
    );
    assert!(index_state_before.free_pads.is_empty());

    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_after.pending_verification_pads.is_empty(),
        "Pending list should be empty after purge"
    );
    assert_eq!(
        index_state_after.free_pads.len(),
        num_existing,
        "Incorrect number of pads moved to free list (only existing should move)"
    );

    let final_free_addresses: std::collections::HashSet<_> = index_state_after
        .free_pads
        .iter()
        .map(|(addr, _, _)| *addr)
        .collect();
    assert_eq!(
        final_free_addresses, expected_free_addresses,
        "Addresses in free list do not match expected addresses"
    );

    for (_, _, counter) in &index_state_after.free_pads {
        assert_eq!(*counter, 0, "Pad counter in free list should be 0");
    }
}

#[tokio::test]
async fn test_purge_empty_list() {
    let (_network_adapter, index_manager, pad_lifecycle_manager, _master_key, _master_addr) =
        setup_test_components_with_initialized_index().await;

    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_before.pending_verification_pads.is_empty(),
        "Initial pending list should be empty for this test"
    );
    assert!(
        index_state_before.free_pads.is_empty(),
        "Initial free list should be empty"
    );

    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    assert!(
        purge_result.is_ok(),
        "purge failed for empty list: {:?}",
        purge_result.err()
    );

    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_after.pending_verification_pads.is_empty(),
        "Pending list should remain empty after purging empty list"
    );
    assert!(
        index_state_after.free_pads.is_empty(),
        "Free list should remain empty after purging empty list"
    );
}
