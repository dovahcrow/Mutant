use autonomi::{Network, Wallet};
use log::{error, info, warn};
use mutant_lib::error::Error;
use mutant_lib::events::{GetCallback, InitCallback};
use serial_test::serial;

// --- Test Initialization Helper ---
async fn setup_test_env() -> Result<(mutant_lib::mutant::MutAnt, Network, String), Error> {
    // Add a small delay before network init
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let network = Network::new(true).map_err(|e| Error::NetworkConnectionFailed(e.to_string()))?;

    // Restore the pre-funded key constant
    const PRE_FUNDED_PK: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let private_key_hex = PRE_FUNDED_PK.to_string();

    // Remove key generation logic
    // let private_key = SecretKey::random();
    // let private_key_hex = hex::encode(private_key.to_bytes());
    // info!("Generated temporary PK for test: ...{}", &private_key_hex[private_key_hex.len()-6..]);

    const MAX_RETRIES: u32 = 3;
    const RETRY_DELAY_MS: u64 = 2500;
    let mut last_error: Option<Error> = None;

    for attempt in 0..MAX_RETRIES {
        info!(
            "Setup attempt {}/{}: Initializing MutAnt (including Storage)...",
            attempt + 1,
            MAX_RETRIES
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create wallet from the pre-funded key hex string
        let wallet =
            Wallet::new_from_private_key(network.clone(), &private_key_hex).map_err(|e| {
                Error::WalletCreationFailed(format!(
                    "Failed to create wallet from pre-funded key: {}",
                    e
                ))
            })?;

        // Call the new MutAnt::new directly
        match mutant_lib::mutant::MutAnt::init(
            wallet.clone(),
            private_key_hex.clone(),
            None::<InitCallback>,
        )
        .await
        {
            // Destructure the new return tuple
            Ok((mutant_instance, _maybe_handle)) => {
                info!(
                    "MutAnt initialized successfully on attempt {}.",
                    attempt + 1
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                // Return the MutAnt instance directly
                return Ok((mutant_instance, network, private_key_hex));
            }
            Err(e) => {
                error!("MutAnt init attempt {} failed: {}", attempt + 1, e);
                // Retry logic remains the same
                if (e.to_string().contains("Network error")
                    || e.to_string().contains("Failed to fetch/decrypt vault"))
                    && attempt < MAX_RETRIES - 1
                {
                    last_error = Some(e);
                    warn!(
                        "Retrying MutAnt initialization after {}ms...",
                        RETRY_DELAY_MS
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| Error::InternalError("Setup failed after retries".to_string())))
}

// --- Granular Tests ---

#[tokio::test]
#[serial]
async fn test_store_and_fetch_small() -> Result<(), Error> {
    info!("--- Test: Store and Fetch Small Item ---");
    // Adjust destructuring for the new setup_test_env return type
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    // Prefix key with test name for isolation
    let key = "small_test_small_key".to_string();
    let value = b"Small test value".to_vec();

    mutant_instance.store(key.clone(), &value, None).await?;
    info!("Stored key: {}, value_len: {}", key, value.len());
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    let fetched_value: Vec<u8> = mutant_instance.fetch(&key, None::<GetCallback>).await?;
    assert_eq!(
        fetched_value, value,
        "Fetched value does not match stored value"
    );
    info!("Fetched key: {}, value_len: {}", key, fetched_value.len());

    info!("Test test_store_and_fetch_small completed successfully.");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_store_and_fetch_large() -> Result<(), Error> {
    info!("--- Test: Store and Fetch Large Item ---");
    // Adjust destructuring
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    // Prefix key with test name for isolation
    let key = "large_test_large_key".to_string();
    let value = vec![1u8; 6 * 1024 * 1024]; // 6MB

    mutant_instance.store(key.clone(), &value, None).await?;
    info!("Stored key: {}, value_len: {}", key, value.len());
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let fetched_value: Vec<u8> = mutant_instance.fetch(&key, None::<GetCallback>).await?;
    assert_eq!(
        fetched_value, value,
        "Fetched large value does not match stored value"
    );
    info!("Fetched key: {}, value_len: {}", key, fetched_value.len());

    info!("Test test_store_and_fetch_large completed successfully.");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_update_item() -> Result<(), Error> {
    info!("--- Test: Update Item ---");
    // Adjust destructuring
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    // Prefix key with test name for isolation
    let key = "update_test_update_key".to_string();
    let initial_value = b"Initial value".to_vec();
    let updated_value = b"Updated value".to_vec();

    // Store initial value
    mutant_instance
        .store(key.clone(), &initial_value, None)
        .await?;
    info!(
        "Stored initial key: {}, value_len: {}",
        key,
        initial_value.len()
    );
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    // Update the value
    mutant_instance
        .update(key.clone(), &updated_value, None)
        .await?;
    info!(
        "Updated key: {}, new_value_len: {}",
        key,
        updated_value.len()
    );
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    // Fetch and verify updated value
    let fetched_updated_value: Vec<u8> = mutant_instance.fetch(&key, None::<GetCallback>).await?;
    assert_eq!(
        fetched_updated_value, updated_value,
        "Fetched updated value does not match"
    );
    info!(
        "Fetched updated key: {}, value_len: {}",
        key,
        fetched_updated_value.len()
    );

    info!("Test test_update_item completed successfully.");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_remove_item() -> Result<(), Error> {
    info!("--- Test: Remove Item ---");
    // Adjust destructuring
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    // Prefix keys with test name for isolation
    let key_to_remove = "remove_test_remove_key".to_string();
    let value_to_remove = b"Value to be removed".to_vec();
    let key_to_keep = "remove_test_keep_key".to_string();
    let value_to_keep = b"Value to keep".to_vec();

    // Store both items
    mutant_instance
        .store(key_to_remove.clone(), &value_to_remove, None)
        .await?;
    info!("Stored key to remove: {}", key_to_remove);
    mutant_instance
        .store(key_to_keep.clone(), &value_to_keep, None)
        .await?;
    info!("Stored key to keep: {}", key_to_keep);
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    // Remove the first item
    mutant_instance.remove(&key_to_remove).await?;
    info!("Removed key: {}", key_to_remove);
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    // Verify removal of the first item
    match mutant_instance
        .fetch(&key_to_remove, None::<GetCallback>)
        .await
    {
        Err(Error::KeyNotFound(_)) => {
            info!("Verified key '{}' is removed.", key_to_remove);
        }
        Ok(_) => panic!("Key '{}' was found after removal.", key_to_remove),
        Err(e) => panic!(
            "Unexpected error fetching removed key '{}': {}",
            key_to_remove, e
        ),
    }

    // Verify the second item still exists
    let fetched_kept_value: Vec<u8> = mutant_instance
        .fetch(&key_to_keep, None::<GetCallback>)
        .await?;
    assert_eq!(
        fetched_kept_value, value_to_keep,
        "Kept value changed after removing another key"
    );
    info!("Verified key '{}' still exists.", key_to_keep);

    info!("Test test_remove_item completed successfully.");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_persistence() -> Result<(), Error> {
    info!("--- Test: Persistence ---");
    // Use setup_test_env to get network and key, but we'll create the instances inside
    let (_initial_mutant, network, private_key_hex) = setup_test_env().await?;
    info!("Initial setup for persistence test completed (network/key obtained).");

    // Prefix keys with test name for isolation
    let key = "persist_test_persistence_key".to_string();
    let value = b"Persistent value".to_vec();
    let key_removed = "persist_test_removed_persistence_key".to_string();
    let value_removed = b"This should not persist".to_vec();

    // Use the initial instance to store and remove
    {
        // Create the first instance
        let wallet_initial = Wallet::new_from_private_key(network.clone(), &private_key_hex)
            .map_err(|e| Error::WalletCreationFailed(e.to_string()))?;
        let (mutant_initial, _handle_initial) = mutant_lib::mutant::MutAnt::init(
            wallet_initial,
            private_key_hex.clone(),
            None::<InitCallback>,
        )
        .await?;
        info!("Created first MutAnt instance for persistence test.");

        // Use the destructured mutant_initial
        mutant_initial.store(key.clone(), &value, None).await?;
        info!("Stored persistent key: {}", key);
        mutant_initial
            .store(key_removed.clone(), &value_removed, None)
            .await?;
        info!(
            "Stored key to be removed before recreation: {}",
            key_removed
        );
        mutant_initial.remove(&key_removed).await?;
        info!("Removed key before recreation: {}", key_removed);
    } // mutant_initial goes out of scope

    info!("Re-creating MutAnt instance with the same private key...");
    // Recreate wallet and the second MutAnt instance
    let wallet_recreate = Wallet::new_from_private_key(network.clone(), &private_key_hex)
        .map_err(|e| Error::WalletCreationFailed(e.to_string()))?;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    let (mutant_recreate, _handle_recreate) =
        mutant_lib::mutant::MutAnt::init(wallet_recreate, private_key_hex, None::<InitCallback>)
            .await
            .expect("Failed to recreate mutant");
    info!("MutAnt instance recreated.");

    // Fetch the persisted key from the recreated instance
    // Use the destructured mutant_recreated
    let fetched_value_recreated: Vec<u8> = mutant_recreate.fetch(&key, None::<GetCallback>).await?;
    assert_eq!(
        fetched_value_recreated, value,
        "Value was not persisted across instances"
    );
    info!("Verified key '{}' persisted across instances.", key);

    // Verify the removed key did not reappear
    match mutant_recreate
        .fetch(&key_removed, None::<GetCallback>)
        .await
    {
        Err(Error::KeyNotFound(_)) => {
            info!("Verified removed key '{}' did not reappear.", key_removed);
        }
        Ok(_) => panic!("Removed key '{}' reappeared after recreation.", key_removed),
        Err(e) => panic!(
            "Unexpected error fetching removed key '{}' after recreation: {}",
            key_removed, e
        ),
    }

    info!("Test test_persistence completed successfully.");
    Ok(())
}
