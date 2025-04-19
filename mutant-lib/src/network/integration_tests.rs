#![cfg(test)]

use crate::data::PRIVATE_DATA_ENCODING;
use crate::index::structure::PadStatus;
use crate::network::adapter::create_public_scratchpad;
use crate::network::adapter::AutonomiNetworkAdapter;
use crate::network::{NetworkChoice, NetworkError};
use autonomi::Bytes;
use autonomi::{ScratchpadAddress, SecretKey};
use serial_test::serial;

const DEV_TESTNET_PRIVATE_KEY_HEX: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn setup_adapter() -> AutonomiNetworkAdapter {
    AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet)
        .expect("Test adapter setup failed")
}

#[tokio::test]
async fn test_adapter_new_success() {
    let result = AutonomiNetworkAdapter::new(DEV_TESTNET_PRIVATE_KEY_HEX, NetworkChoice::Devnet);
    assert!(
        result.is_ok(),
        "Adapter creation failed: {:?}",
        result.err()
    );
    let adapter = result.unwrap();
    assert_eq!(
        adapter.get_network_choice(),
        NetworkChoice::Devnet,
        "Network choice mismatch"
    );
    let wallet_ref = adapter.wallet();
    assert!(
        !wallet_ref.address().is_empty(),
        "Wallet address seems empty"
    );

    let check_key = SecretKey::random();
    let check_addr = ScratchpadAddress::new(check_key.public_key());
    let check_exist_res = adapter.check_existence(&check_addr).await;
    assert!(
        check_exist_res.is_ok(),
        "Implicit client connection failed: {:?}",
        check_exist_res.err()
    );
    assert!(
        !check_exist_res.unwrap(),
        "Randomly generated key should not exist"
    );
}

#[tokio::test]
async fn test_adapter_new_invalid_key() {
    let invalid_key = "0xinvalidhexkey";
    let result = AutonomiNetworkAdapter::new(invalid_key, NetworkChoice::Devnet);
    assert!(
        result.is_err(),
        "Adapter creation should fail for invalid key"
    );

    match result.err().unwrap() {
        NetworkError::InvalidKeyInput(_) => {}
        e => panic!(
            "Expected InvalidKeyInput due to hex decode error, but got {:?}",
            e
        ),
    }
}

#[tokio::test]
async fn test_check_existence_nonexistent() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();
    let test_addr = ScratchpadAddress::new(test_key.public_key());

    let exists = adapter.check_existence(&test_addr).await;
    assert!(exists.is_ok(), "check_existence failed: {:?}", exists.err());
    assert!(
        !exists.unwrap(),
        "Expected randomly generated scratchpad {} to not exist",
        test_addr
    );
}

#[tokio::test]
async fn test_put_raw_create_and_check() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();
    let expected_addr = ScratchpadAddress::new(test_key.public_key());
    let test_data = b"create_and_check data";

    let put_result = adapter
        .put_raw(
            &test_key,
            test_data,
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await;
    assert!(put_result.is_ok(), "put_raw failed: {:?}", put_result.err());
    assert_eq!(
        put_result.unwrap(),
        expected_addr,
        "put_raw address mismatch"
    );

    let exists = adapter.check_existence(&expected_addr).await;
    assert!(exists.is_ok(), "check_existence failed: {:?}", exists.err());
    assert!(
        exists.unwrap(),
        "Expected pad {} to exist after put_raw",
        expected_addr
    );
}

#[tokio::test]
async fn test_get_raw_scratchpad() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();
    let test_addr = ScratchpadAddress::new(test_key.public_key());
    let test_data = b"get_raw_scratchpad data";

    adapter
        .put_raw(
            &test_key,
            test_data,
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await
        .expect("put_raw failed during setup");

    let get_result = adapter.get_raw_scratchpad(&test_addr).await;
    assert!(
        get_result.is_ok(),
        "get_raw_scratchpad failed: {:?}",
        get_result.err()
    );
}

#[tokio::test]
async fn test_put_raw_update() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();
    let test_addr = ScratchpadAddress::new(test_key.public_key());
    let data1 = b"initial update data";
    let data2 = b"updated data indeed";

    adapter
        .put_raw(
            &test_key,
            data1,
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await
        .expect("put_raw (create) failed during setup");

    let update_result = adapter
        .put_raw(&test_key, data2, &PadStatus::Written, PRIVATE_DATA_ENCODING)
        .await;
    assert!(
        update_result.is_ok(),
        "put_raw (update) failed: {:?}",
        update_result.err()
    );
    assert_eq!(
        update_result.unwrap(),
        test_addr,
        "Update returned wrong address"
    );

    let get_result = adapter.get_raw_scratchpad(&test_addr).await;
    assert!(
        get_result.is_ok(),
        "get_raw_scratchpad after update failed: {:?}",
        get_result.err()
    );
}

#[tokio::test]
async fn test_put_raw_create_fails_if_exists() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();
    let test_data = b"create fail test data";

    adapter
        .put_raw(
            &test_key,
            test_data,
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await
        .expect("put_raw (create) failed during setup");

    let result = adapter
        .put_raw(
            &test_key,
            b"different data",
            &PadStatus::Generated,
            PRIVATE_DATA_ENCODING,
        )
        .await;

    assert!(result.is_err(), "Second create should fail");
    match result.err().unwrap() {
        NetworkError::InconsistentState(_) => {}
        e => panic!("Expected InconsistentState, got {:?}", e),
    }
}

#[tokio::test]
async fn test_put_raw_update_fails_if_not_exists() {
    let adapter = setup_adapter().await;
    let test_key = SecretKey::random();

    let result = adapter
        .put_raw(
            &test_key,
            b"update fail test data",
            &PadStatus::Written,
            PRIVATE_DATA_ENCODING,
        )
        .await;

    assert!(result.is_err(), "Update should fail for non-existent pad");
    match result.err().unwrap() {
        NetworkError::InconsistentState(_) => {}
        e => panic!("Expected InconsistentState, got {:?}", e),
    }
}
