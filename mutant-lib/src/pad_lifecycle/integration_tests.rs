#![cfg(test)]

use crate::index::error::IndexError;
use crate::index::manager::{DefaultIndexManager, IndexManager};
use crate::index::structure::{KeyInfo, PadInfo, PadStatus};
use crate::network::adapter::{AutonomiNetworkAdapter, NetworkAdapter};
use crate::network::NetworkChoice;
use crate::pad_lifecycle::error::PadLifecycleError;
use crate::pad_lifecycle::manager::{DefaultPadLifecycleManager, PadLifecycleManager};
use crate::pad_lifecycle::PadOrigin;
use crate::storage::manager::{DefaultStorageManager, StorageManager};
use autonomi::{ScratchpadAddress, SecretKey};
use std::sync::Arc;
use tokio::runtime::Runtime;

// Use a known devnet key for setting up adapters/managers consistently
const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

// --- Test Setup Helper ---

/// Creates the components needed for PadLifecycleManager tests.
/// Also initializes the IndexManager with a random master key to simulate a real environment.
async fn setup_test_components_with_initialized_index() -> (
    Arc<dyn NetworkAdapter>,
    Arc<dyn StorageManager>,
    Arc<DefaultIndexManager>,
    DefaultPadLifecycleManager,
    SecretKey, // Return master key for potential use in tests
    ScratchpadAddress,
) {
    let network_adapter: Arc<dyn NetworkAdapter> = Arc::new(
        AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
            .expect("Test NetworkAdapter setup failed"),
    );
    let storage_manager: Arc<dyn StorageManager> =
        Arc::new(DefaultStorageManager::new(network_adapter.clone()));
    let index_manager = Arc::new(DefaultIndexManager::new(
        storage_manager.clone(),
        network_adapter.clone(),
    ));

    // Generate random master key/address for index isolation
    let master_key = SecretKey::random();
    let master_addr = ScratchpadAddress::new(master_key.public_key());

    // Initialize the index manager (load or create default)
    index_manager
        .load_or_initialize(&master_addr, &master_key)
        .await
        .expect("Index manager initialization failed");

    let pad_lifecycle_manager = DefaultPadLifecycleManager::new(
        index_manager.clone(),
        network_adapter.clone(),
        storage_manager.clone(),
    );

    (
        network_adapter,
        storage_manager,
        index_manager,
        pad_lifecycle_manager,
        master_key,
        master_addr,
    )
}

// --- Tests ---

#[test]
fn test_acquire_new_pads() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let (
            _network_adapter,
            _storage_manager,
            index_manager,
            pad_lifecycle_manager,
            _master_key,
            _master_addr,
        ) = setup_test_components_with_initialized_index().await;

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

#[tokio::test] // Marked as async test
async fn test_acquire_pads_from_free_pool() {
    let (
        _network_adapter,
        storage_manager, // Need storage_manager to write pads
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        _master_addr,
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Add pads to the free pool ---
    let num_pads_to_add = 3;
    let mut free_pads_info = Vec::new();
    for i in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        // Write dummy data to ensure pad exists on network (counter will be 0)
        storage_manager
            .write_pad_data(&key, &[0u8; 10], &PadStatus::Generated) // Use Generated status for creation
            .await
            .unwrap_or_else(|e| panic!("Failed to write dummy data for pad {}: {}", i, e));

        free_pads_info.push((address, key_bytes));
    }

    // Add pads to index manager's free list (will fetch counter 0)
    index_manager
        .add_free_pads(free_pads_info.clone())
        .await // Clone needed as we use it later
        .expect("Failed to add pads to free list");

    // --- Action ---
    let num_pads_to_acquire = 2;
    let mut callback = None;
    let result = pad_lifecycle_manager
        .acquire_pads(num_pads_to_acquire, &mut callback)
        .await;

    // --- Assertions ---
    assert!(result.is_ok(), "acquire_pads failed: {:?}", result.err());
    let acquired_pads = result.unwrap();
    assert_eq!(
        acquired_pads.len(),
        num_pads_to_acquire,
        "Incorrect number of pads acquired"
    );

    // Verify the origin and counter of acquired pads
    let mut acquired_addresses = std::collections::HashSet::new();
    for (address, _key, origin) in &acquired_pads {
        assert_eq!(
            *origin,
            PadOrigin::FreePool { initial_counter: 0 }, // Expect counter 0 from initial write
            "Acquired pad should have FreePool origin with counter 0"
        );
        acquired_addresses.insert(*address);
    }

    // Verify IndexManager state
    let index_state_after = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_after.free_pads.len(),
        num_pads_to_add - num_pads_to_acquire, // Should be 1 remaining
        "Incorrect number of pads remaining in free pool"
    );
    // Ensure the remaining free pad is not one of the acquired ones
    assert!(!index_state_after.free_pads.is_empty());
    let remaining_free_addr = index_state_after.free_pads[0].0;
    assert!(!acquired_addresses.contains(&remaining_free_addr));

    // Check that KeyInfo map is still empty
    assert!(
        index_state_after.index.is_empty(),
        "Index map should remain empty"
    );
}

#[tokio::test]
async fn test_acquire_mixed_pads() {
    let (
        _network_adapter,
        storage_manager,
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        _master_addr,
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Add 2 pads to the free pool ---
    let num_free_pads_to_add = 2;
    let mut free_pads_info = Vec::new();
    for i in 0..num_free_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        storage_manager
            .write_pad_data(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .unwrap_or_else(|e| panic!("Failed to write dummy data for pad {}: {}", i, e));
        free_pads_info.push((address, key_bytes));
    }
    index_manager
        .add_free_pads(free_pads_info.clone())
        .await
        .expect("Failed to add pads to free list");

    // --- Action ---
    let num_pads_to_acquire = 3; // Request more than available in free pool
    let mut callback = None;
    let result = pad_lifecycle_manager
        .acquire_pads(num_pads_to_acquire, &mut callback)
        .await;

    // --- Assertions ---
    assert!(result.is_ok(), "acquire_pads failed: {:?}", result.err());
    let acquired_pads = result.unwrap();
    assert_eq!(
        acquired_pads.len(),
        num_pads_to_acquire,
        "Incorrect number of pads acquired"
    );

    // Verify origins: 2 FreePool, 1 Generated
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

    // Verify IndexManager state
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
    let (
        _network_adapter,
        storage_manager,
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        master_addr, // Need master_addr for network choice resolution if needed
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Add existing pads to the pending list ---
    let num_pads_to_add = 2;
    let mut pending_pads_info = Vec::new();
    let mut expected_free_addresses = std::collections::HashSet::new();

    for i in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        // Write the pad to the network so it exists
        storage_manager
            .write_pad_data(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .unwrap_or_else(|e| panic!("Failed to write data for pending pad {}: {}", i, e));

        pending_pads_info.push((address, key_bytes));
        expected_free_addresses.insert(address);
    }

    // Add pads to index manager's pending list
    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    // Verify initial state
    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_pads_to_add
    );
    assert!(index_state_before.free_pads.is_empty());

    // --- Action ---
    // Use Devnet as network choice for cache path resolution
    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    // --- Assertions ---
    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    // Verify IndexManager state after purge
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

    // Check that the correct pads are in the free list
    let final_free_addresses: std::collections::HashSet<_> = index_state_after
        .free_pads
        .iter()
        .map(|(addr, _, _)| *addr)
        .collect();
    assert_eq!(
        final_free_addresses, expected_free_addresses,
        "Addresses in free list do not match expected addresses"
    );

    // Check counter is 0 (from initial write)
    for (_, _, counter) in &index_state_after.free_pads {
        assert_eq!(*counter, 0, "Pad counter in free list should be 0");
    }
}

#[tokio::test]
async fn test_purge_non_existent_pads() {
    let (
        _network_adapter,
        _storage_manager,
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        _master_addr,
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Add non-existent pads to the pending list ---
    let num_pads_to_add = 2;
    let mut pending_pads_info = Vec::new();

    for _i in 0..num_pads_to_add {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();

        // DO NOT write these pads to the network

        pending_pads_info.push((address, key_bytes));
    }

    // Add pads to index manager's pending list
    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    // Verify initial state
    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_pads_to_add
    );
    assert!(index_state_before.free_pads.is_empty());

    // --- Action ---
    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    // --- Assertions ---
    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    // Verify IndexManager state after purge
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
    let (
        _network_adapter,
        storage_manager,
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        _master_addr,
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Add mixed pads to the pending list ---
    let num_existing = 2;
    let num_non_existent = 3;
    let mut pending_pads_info = Vec::new();
    let mut expected_free_addresses = std::collections::HashSet::new();

    // Add existing pads
    for i in 0..num_existing {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        storage_manager
            .write_pad_data(&key, &[0u8; 10], &PadStatus::Generated)
            .await
            .unwrap_or_else(|e| panic!("Failed to write data for existing pad {}: {}", i, e));
        pending_pads_info.push((address, key_bytes));
        expected_free_addresses.insert(address);
    }

    // Add non-existent pads
    for _i in 0..num_non_existent {
        let key = SecretKey::random();
        let address = ScratchpadAddress::new(key.public_key());
        let key_bytes = key.to_bytes().to_vec();
        pending_pads_info.push((address, key_bytes));
    }

    // Add all pads to index manager's pending list
    index_manager
        .add_pending_pads(pending_pads_info)
        .await
        .expect("Failed to add pads to pending list");

    // Verify initial state
    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert_eq!(
        index_state_before.pending_verification_pads.len(),
        num_existing + num_non_existent
    );
    assert!(index_state_before.free_pads.is_empty());

    // --- Action ---
    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    // --- Assertions ---
    assert!(
        purge_result.is_ok(),
        "purge failed: {:?}",
        purge_result.err()
    );

    // Verify IndexManager state after purge
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

    // Check that the correct pads are in the free list
    let final_free_addresses: std::collections::HashSet<_> = index_state_after
        .free_pads
        .iter()
        .map(|(addr, _, _)| *addr)
        .collect();
    assert_eq!(
        final_free_addresses, expected_free_addresses,
        "Addresses in free list do not match expected addresses"
    );

    // Check counter is 0 (from initial write)
    for (_, _, counter) in &index_state_after.free_pads {
        assert_eq!(*counter, 0, "Pad counter in free list should be 0");
    }
}

#[tokio::test]
async fn test_purge_empty_list() {
    let (
        _network_adapter,
        _storage_manager,
        index_manager,
        pad_lifecycle_manager,
        _master_key,
        _master_addr,
    ) = setup_test_components_with_initialized_index().await;

    // --- Setup: Verify pending list is empty ---
    let index_state_before = index_manager.get_index_copy().await.unwrap();
    assert!(
        index_state_before.pending_verification_pads.is_empty(),
        "Initial pending list should be empty for this test"
    );
    assert!(
        index_state_before.free_pads.is_empty(),
        "Initial free list should be empty"
    );

    // --- Action ---
    let purge_result = pad_lifecycle_manager
        .purge(None, NetworkChoice::Devnet)
        .await;

    // --- Assertions ---
    assert!(
        purge_result.is_ok(),
        "purge failed for empty list: {:?}",
        purge_result.err()
    );

    // Verify IndexManager state after purge
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

// TODO: Add test for purging with network errors
// TODO: Add test for acquire_pads failure (e.g., network error during creation)
// TODO: Add tests for import_free_pad
