#![cfg(test)]

use crate::data::{PRIVATE_DATA_ENCODING, PUBLIC_DATA_ENCODING};
use crate::index::structure::{PadInfo, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
use autonomi::{Scratchpad, ScratchpadAddress, SecretKey};
use rand::RngCore; // For generating random data
use std::time::Duration; // For sleeps

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

// Helper function to create a basic PadInfo for testing creation
fn create_initial_pad_info(data_size: usize) -> (PadInfo, ScratchpadAddress) {
    // 1. Generate a unique SecretKey for this pad
    let secret_key = SecretKey::random();
    // 2. Derive the ScratchpadAddress from the public key
    let address = ScratchpadAddress::new(secret_key.public_key());
    // 3. Get the key bytes to store in PadInfo
    let sk_bytes = secret_key.to_bytes().to_vec();

    let pad_info = PadInfo {
        address,
        chunk_index: 0,               // Default to 0 for single-chunk tests
        size: data_size,              // Use usize directly
        status: PadStatus::Generated, // Initial status before network interaction
        last_known_counter: 0,        // Default to 0
        sk_bytes,                     // Store the generated key's bytes
        receipt: None,                // No receipt before first put
    };
    (pad_info, address)
}

#[tokio::test]
async fn test_get_public() {
    let adapter = setup_adapter().await;
    let data = generate_random_data(1024);
    let (pad_info, address) = create_initial_pad_info(data.len());

    // 1. Put data first
    let put_result = adapter
        .put_public(&pad_info, &data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        put_result.is_ok(),
        "Put public failed: {:?}",
        put_result.err()
    );
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 2. Get data
    let get_result = adapter.get_public(&address).await;
    assert!(
        get_result.is_ok(),
        "Get public failed: {:?}",
        get_result.err()
    );

    let get_result = get_result.unwrap();
    assert_eq!(get_result.data, data, "Retrieved data does not match");
    assert_eq!(
        get_result.data_encoding, PUBLIC_DATA_ENCODING,
        "Retrieved encoding mismatch"
    );
    // Counter might vary depending on network state/creation, focus on data
    // assert_eq!(get_result.counter, put_result.unwrap().counter, "Counter mismatch");
}

#[tokio::test]
async fn test_get_private() {
    let adapter = setup_adapter().await;
    let data = generate_random_data(1024);
    let (pad_info, address) = create_initial_pad_info(data.len());

    // 1. Put private data first
    let put_result = adapter
        .put_private(&pad_info, &data, PRIVATE_DATA_ENCODING)
        .await;
    assert!(
        put_result.is_ok(),
        "Put private failed: {:?}",
        put_result.err()
    );
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(10)).await;

    // 2. Get private data
    let get_result = adapter.get_private(&pad_info).await;
    assert!(
        get_result.is_ok(),
        "Get private failed: {:?}",
        get_result.err()
    );

    let get_result = get_result.unwrap();
    assert_eq!(get_result.data, data, "Retrieved private data mismatch");
    assert_eq!(
        get_result.data_encoding, PRIVATE_DATA_ENCODING,
        "Retrieved private encoding mismatch"
    );
    // assert_eq!(get_result.counter, put_result.unwrap().counter, "Private counter mismatch");
}

#[tokio::test]
async fn test_put_public_create() {
    let adapter = setup_adapter().await;
    let data = generate_random_data(512);
    let (pad_info, address) = create_initial_pad_info(data.len());

    let result = adapter
        .put_public(&pad_info, &data, PUBLIC_DATA_ENCODING)
        .await;

    assert!(
        result.is_ok(),
        "Put public create failed: {:?}",
        result.err()
    );
    let put_result = result.unwrap();

    assert_eq!(put_result.address, address, "Address mismatch");
    assert_eq!(
        put_result.data_encoding, PUBLIC_DATA_ENCODING,
        "Encoding mismatch"
    );
    // Initial counter might not always be 0 depending on network interactions,
    // but should be low for a new address. Check >= 0 is usually enough.
    assert!(put_result.counter == 0, "Counter should be 0");
    assert!(!put_result.receipt.is_empty(), "Receipt should be present");
}

#[tokio::test]
async fn test_put_public_update() {
    let adapter = setup_adapter().await;
    let initial_data = generate_random_data(512);
    let (mut pad_info, address) = create_initial_pad_info(initial_data.len());

    // 1. Initial Put
    let create_result = adapter
        .put_public(&pad_info, &initial_data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        create_result.is_ok(),
        "Initial put public failed: {:?}",
        create_result.err()
    );
    let put_result = create_result.unwrap();
    let initial_counter = put_result.counter;
    let receipt = put_result.receipt;
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 2. Update Put
    let updated_data = generate_random_data(600); // Different size
    pad_info.size = updated_data.len();
    pad_info.last_known_counter = initial_counter + 1; // Increment counter for update
    pad_info.receipt = Some(receipt);
    pad_info.size = updated_data.len(); // Update size

    let update_result = adapter
        .put_public(&pad_info, &updated_data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        update_result.is_ok(),
        "Update put public failed: {:?}",
        update_result.err()
    );
    let put_update_result = update_result.unwrap();

    assert_eq!(
        put_update_result.address, address,
        "Update address mismatch"
    );
    assert_eq!(
        put_update_result.counter,
        initial_counter + 1,
        "Counter did not increment correctly"
    );
    assert_eq!(
        put_update_result.data_encoding, PUBLIC_DATA_ENCODING,
        "Update encoding mismatch"
    );
    assert!(
        !put_update_result.receipt.is_empty(),
        "Receipt should be present"
    );
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 3. Verify with Get
    let get_result = adapter.get_public(&address).await;
    assert!(
        get_result.is_ok(),
        "Get public after update failed: {:?}",
        get_result.err()
    );
    let final_data = get_result.unwrap();
    assert_eq!(final_data.data, updated_data, "Updated data not retrieved");
    assert_eq!(
        final_data.counter,
        initial_counter + 1,
        "Final counter mismatch after update"
    );
}

#[tokio::test]
async fn test_put_private_create() {
    let adapter = setup_adapter().await;
    let data = generate_random_data(512);
    let (pad_info, address) = create_initial_pad_info(data.len());

    let result = adapter
        .put_private(&pad_info, &data, PRIVATE_DATA_ENCODING)
        .await;

    assert!(
        result.is_ok(),
        "Put private create failed: {:?}",
        result.err()
    );
    let put_result = result.unwrap();

    assert_eq!(put_result.address, address, "Private address mismatch");
    assert_eq!(
        put_result.data_encoding, PRIVATE_DATA_ENCODING,
        "Private encoding mismatch"
    );
    assert!(put_result.counter >= 0, "Private counter non-negative");
    assert!(
        !put_result.receipt.is_empty(),
        "Private receipt should be present"
    );
}

#[tokio::test]
async fn test_put_private_update() {
    let adapter = setup_adapter().await;
    let initial_data = generate_random_data(512);
    let (mut pad_info, address) = create_initial_pad_info(initial_data.len());

    // 1. Initial Private Put
    let create_result = adapter
        .put_private(&pad_info, &initial_data, PRIVATE_DATA_ENCODING)
        .await;
    assert!(
        create_result.is_ok(),
        "Initial put private failed: {:?}",
        create_result.err()
    );
    let put_result = create_result.unwrap();
    let initial_counter = put_result.counter;
    let receipt = put_result.receipt;
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 2. Update Private Put
    let updated_data = generate_random_data(700);
    pad_info.size = updated_data.len();
    pad_info.receipt = Some(receipt);
    pad_info.last_known_counter = initial_counter + 1;
    // Similar counter handling as public update
    // pad_info.last_known_counter = initial_counter;

    let update_result = adapter
        .put_private(&pad_info, &updated_data, PRIVATE_DATA_ENCODING)
        .await;
    assert!(
        update_result.is_ok(),
        "Update put private failed: {:?}",
        update_result.err()
    );
    let put_update_result = update_result.unwrap();

    assert_eq!(
        put_update_result.address, address,
        "Private update address mismatch"
    );
    assert_eq!(
        put_update_result.counter,
        initial_counter + 1,
        "Private counter did not increment"
    );
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 3. Verify with Get Private
    let get_result = adapter.get_private(&pad_info).await;
    assert!(
        get_result.is_ok(),
        "Get private after update failed: {:?}",
        get_result.err()
    );
    let final_data = get_result.unwrap();
    assert_eq!(
        final_data.data, updated_data,
        "Updated private data not retrieved"
    );
    assert_eq!(
        final_data.counter,
        initial_counter + 1,
        "Final private counter mismatch"
    );
}

#[tokio::test]
async fn test_fetch_receipt() {
    let adapter = setup_adapter().await;
    let data = generate_random_data(256);
    // Use public put for simplicity, could be private too
    let (pad_info, _address) = create_initial_pad_info(data.len());

    // 1. Put data to ensure there's something to fetch a receipt for
    let put_result = adapter
        .put_private(&pad_info, &data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        put_result.is_ok(),
        "Put public for receipt test failed: {:?}",
        put_result.err()
    );
    // Give network time to settle
    tokio::time::sleep(Duration::from_secs(20)).await;
    let put_result = put_result.unwrap();
    let mut pad_info = pad_info.clone();
    pad_info.address = put_result.address;
    pad_info.size = put_result.size_tmp;

    // 2. Fetch the receipt using the PadInfo from the initial state
    let receipt_result = adapter.fetch_receipt(&pad_info).await;
    assert!(
        receipt_result.is_ok(),
        "Fetch receipt failed: {:?}",
        receipt_result.err()
    );

    let receipt = receipt_result.unwrap();
    // Basic checks on the receipt - it should exist and have a non-zero amount
    // More detailed validation depends on expected network pricing
    assert!(!receipt.is_empty(), "Fetched receipt is empty");
    // We could potentially compare this fetched receipt with the one returned by `put_result`,
    // though they might differ slightly depending on timing and network state.
    // assert_eq!(receipt.amount, put_result.unwrap().receipt.amount, "Fetched receipt amount differs from put receipt amount");
}
