#![cfg(test)]

use crate::data::{PRIVATE_DATA_ENCODING, PUBLIC_DATA_ENCODING};
use crate::index::structure::{PadInfo, PadStatus};
use crate::network::{AutonomiNetworkAdapter, NetworkChoice};
use autonomi::{AttoTokens, ScratchpadAddress, SecretKey};
use rand::RngCore;

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

fn create_initial_pad_info(data_size: usize) -> (PadInfo, ScratchpadAddress) {
    let secret_key = SecretKey::random();
    let address = ScratchpadAddress::new(secret_key.public_key());
    let sk_bytes = secret_key.to_bytes().to_vec();

    let pad_info = PadInfo {
        address,
        chunk_index: 0,
        size: data_size,
        status: PadStatus::Generated,
        last_known_counter: 0,
        sk_bytes,
    };
    (pad_info, address)
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
    assert!(
        put_result.cost > AttoTokens::zero(),
        "Initial put should have a cost > 0"
    );
    let get_result = adapter.get_public(&address).await;
    assert!(
        get_result.is_ok(),
        "Get public failed: {:?}",
        get_result.err()
    );

    let get_result = get_result.unwrap();
    assert_eq!(get_result.data, data, "Retrieved private data mismatch");
    assert_eq!(get_result.counter, 0, "Counter should be 0");
    assert_eq!(
        get_result.data_encoding, PUBLIC_DATA_ENCODING,
        "Retrieved public encoding mismatch"
    );
}

#[tokio::test]
async fn test_put_public_update() {
    let adapter = setup_adapter().await;
    let initial_data = generate_random_data(512);
    let (mut pad_info, address) = create_initial_pad_info(initial_data.len());

    let create_result = adapter
        .put_public(&pad_info, &initial_data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        create_result.is_ok(),
        "Initial put public failed: {:?}",
        create_result.err()
    );
    let put_result = create_result.unwrap();

    assert!(
        put_result.cost > AttoTokens::zero(),
        "Initial put should have a cost > 0"
    );

    let initial_counter = pad_info.last_known_counter;

    let updated_data = generate_random_data(600);
    pad_info.size = updated_data.len();
    pad_info.last_known_counter = initial_counter + 1;

    let update_result = adapter
        .put_public(&pad_info, &updated_data, PUBLIC_DATA_ENCODING)
        .await;
    assert!(
        update_result.is_ok(),
        "Update put public failed: {:?}",
        update_result.err()
    );
    let put_update_result = update_result.unwrap();

    assert!(
        put_update_result.cost == AttoTokens::zero(),
        "Update put should have a cost == 0"
    );

    assert_eq!(
        put_update_result.address, address,
        "Update address mismatch"
    );
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

    assert!(
        put_result.cost > AttoTokens::zero(),
        "Initial put should have a cost > 0"
    );

    assert_eq!(put_result.address, address, "Private address mismatch");

    let get_result = adapter.get_private(&pad_info).await;
    assert!(
        get_result.is_ok(),
        "Get private failed: {:?}",
        get_result.err()
    );

    let get_result = get_result.unwrap();
    assert_eq!(get_result.data, data, "Retrieved private data mismatch");
    assert_eq!(get_result.counter, 0, "Counter should be 0");
    assert_eq!(
        get_result.data_encoding, PRIVATE_DATA_ENCODING,
        "Retrieved private encoding mismatch"
    );
}

#[tokio::test]
async fn test_put_private_update() {
    let adapter = setup_adapter().await;
    let initial_data = generate_random_data(512);
    let (mut pad_info, address) = create_initial_pad_info(initial_data.len());

    let create_result = adapter
        .put_private(&pad_info, &initial_data, PRIVATE_DATA_ENCODING)
        .await;
    assert!(
        create_result.is_ok(),
        "Initial put private failed: {:?}",
        create_result.err()
    );
    let put_result = create_result.unwrap();
    assert!(
        put_result.cost > AttoTokens::zero(),
        "Initial put should have a cost > 0"
    );

    let updated_data = generate_random_data(700);
    pad_info.size = updated_data.len();
    pad_info.last_known_counter = 1;

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
        put_update_result.cost,
        AttoTokens::zero(),
        "Update put should have a cost == 0"
    );

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
    assert_eq!(final_data.counter, 1, "Final private counter mismatch");
}
