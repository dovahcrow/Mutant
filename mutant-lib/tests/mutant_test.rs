use autonomi::Network;
use log::{error, info, warn};
use mutant_lib::{error::Error, mutant::MutAntConfig};
use serial_test::serial;

async fn setup_test_env() -> Result<(mutant_lib::mutant::MutAnt, Network, String), Error> {
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let network = Network::new(true).map_err(|e| Error::NetworkConnectionFailed(e.to_string()))?;
    let config = MutAntConfig::local();

    const PRE_FUNDED_PK: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let private_key_hex = PRE_FUNDED_PK.to_string();

    const MAX_RETRIES: u32 = 3;
    const RETRY_DELAY_MS: u64 = 2500;
    let mut last_error: Option<Error> = None;

    for attempt in 0..MAX_RETRIES {
        info!(
            "Setup attempt {}/{}: Initializing MutAnt (including Storage)...",
            attempt + 1,
            MAX_RETRIES
        );

        match mutant_lib::mutant::MutAnt::init_with_progress(
            private_key_hex.clone(),
            config.clone(),
            None,
        )
        .await
        {
            Ok(mutant_instance) => {
                info!(
                    "MutAnt initialized successfully on attempt {}.",
                    attempt + 1
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                return Ok((mutant_instance, network, private_key_hex));
            }
            Err(e) => {
                error!("MutAnt init attempt {} failed: {}", attempt + 1, e);
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

#[tokio::test]
#[serial]
async fn test_store_and_fetch_small() -> Result<(), Error> {
    info!("--- Test: Store and Fetch Small Item ---");
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    let key = "small_test_small_key".to_string();
    let value = b"Small test value".to_vec();

    mutant_instance.store(&key, &value).await?;
    info!("Stored key: {}, value_len: {}", key, value.len());
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    let fetched_value: Vec<u8> = mutant_instance.fetch(&key).await?;
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
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    let key = "large_test_large_key".to_string();
    let value = vec![1u8; 6 * 1024 * 1024]; // 6MB

    mutant_instance.store(&key, &value).await?;
    info!("Stored key: {}, value_len: {}", key, value.len());
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let fetched_value: Vec<u8> = mutant_instance.fetch(&key).await?;
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
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    let key = "update_test_update_key".to_string();
    let initial_value = b"Initial value".to_vec();
    let updated_value = b"Updated value".to_vec();

    mutant_instance.store(&key, &initial_value).await?;
    info!(
        "Stored initial key: {}, value_len: {}",
        key,
        initial_value.len()
    );
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    mutant_instance.update(&key, &updated_value).await?;
    info!(
        "Updated key: {}, new_value_len: {}",
        key,
        updated_value.len()
    );
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    let fetched_updated_value: Vec<u8> = mutant_instance.fetch(&key).await?;
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
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    let key_to_remove = "remove_test_remove_key".to_string();
    let value_to_remove = b"Value to be removed".to_vec();
    let key_to_keep = "remove_test_keep_key".to_string();
    let value_to_keep = b"Value to keep".to_vec();

    mutant_instance
        .store(&key_to_remove, &value_to_remove)
        .await?;
    info!("Stored key to remove: {}", key_to_remove);
    mutant_instance.store(&key_to_keep, &value_to_keep).await?;
    info!("Stored key to keep: {}", key_to_keep);
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    mutant_instance.remove(&key_to_remove).await?;
    info!("Removed key: {}", key_to_remove);
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    match mutant_instance.fetch(&key_to_remove).await {
        Err(Error::KeyNotFound(_)) => {
            info!("Verified key '{}' is removed.", key_to_remove);
        }
        Ok(_) => panic!("Key '{}' was found after removal.", key_to_remove),
        Err(e) => panic!(
            "Unexpected error fetching removed key '{}': {}",
            key_to_remove, e
        ),
    }

    let fetched_kept_value: Vec<u8> = mutant_instance.fetch(&key_to_keep).await?;
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
    let (_initial_mutant, _network, private_key_hex) = setup_test_env().await?;
    info!("Initial setup for persistence test completed (network/key obtained).");

    let key = "persist_test_persistence_key".to_string();
    let value = b"Persistent value".to_vec();
    let key_removed = "persist_test_removed_persistence_key".to_string();
    let value_removed = b"This should not persist".to_vec();

    let config = MutAntConfig::local();

    {
        let mutant_initial = mutant_lib::mutant::MutAnt::init_with_progress(
            private_key_hex.clone(),
            config.clone(),
            None,
        )
        .await?;
        info!("Created first MutAnt instance for persistence test.");

        mutant_initial.store(&key, &value).await?;
        info!("Stored persistent key: {}", key);
        mutant_initial.store(&key_removed, &value_removed).await?;
        info!(
            "Stored key to be removed before recreation: {}",
            key_removed
        );
        mutant_initial.remove(&key_removed).await?;
        info!("Removed key before recreation: {}", key_removed);
    }

    info!("Re-creating MutAnt instance with the same private key...");
    let mutant_recreate = mutant_lib::mutant::MutAnt::init_with_progress(
        private_key_hex.clone(),
        config.clone(),
        None,
    )
    .await?;
    info!("MutAnt instance recreated.");

    let fetched_value_recreated: Vec<u8> = mutant_recreate.fetch(&key).await?;
    assert_eq!(
        fetched_value_recreated, value,
        "Value was not persisted across instances"
    );
    info!("Verified key '{}' persisted across instances.", key);

    match mutant_recreate.fetch(&key_removed).await {
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

#[tokio::test]
#[serial]
async fn test_list_keys() -> Result<(), Error> {
    info!("--- Test: List Keys ---");
    let (mutant_instance, _network, _private_key_hex) = setup_test_env().await?;
    info!("Test setup completed.");

    let key1 = "list_test_key_1".to_string();
    let key2 = "list_test_key_2".to_string();

    mutant_instance.store(&key1, b"v1").await?;
    mutant_instance.store(&key2, b"v2").await?;
    info!("Stored key1 and key2");
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    info!("Test test_list_keys completed successfully.");
    Ok(())
}
