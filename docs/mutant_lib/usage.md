# MutAnt Library: Usage Examples

This document provides practical examples of how to use the `mutant-lib` API.

**Note:** These examples assume you have the necessary `autonomi` setup (wallet, private key) and are running within a `tokio` runtime.

```rust
use mutant_lib::{
    MutAnt,
    Error,
    events::{InitProgressEvent, PutEvent, GetEvent},
    autonomi::{Wallet, Configuration}, // Re-exports
    tokio, // Re-exports
};
use std::sync::Arc;
use std::time::Duration;

// --- Main Async Function ---

async fn run_mutant_examples() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialization
    let wallet = Wallet::from_phrase("your twelve word mnemonic phrase here", None)?;
    let private_key_hex = "your_autonomi_account_private_key_hex_string_here".to_string(); // Ensure this matches the wallet

    // Initialize without callback
    let (mutant, _init_handle) = MutAnt::init(
        wallet.clone(),
        private_key_hex.clone(),
        None, // No callback
    )
    .await?;

    // Wait a moment for potential background init tasks
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 2. Storing Data
    let key1 = "hello.txt".to_string();
    let data1 = b"Hello, MutAnt World!".to_vec();

    // Simple store
    mutant.store(key1.clone(), &data1, None).await?;

    // Store larger data
    let key2 = "large_data.bin".to_string();
    let data2: Vec<u8> = (0..(10 * 1024 * 1024)).map(|i| (i % 256) as u8).collect();

    mutant.store(key2.clone(), &data2, None).await?; // No callback

    // 3. Fetching Data

    // Fetch small data
    let fetched_data1 = mutant.fetch(&key1, None).await?;
    assert_eq!(data1, fetched_data1);

    // Fetch large data
    let fetched_data2 = mutant.fetch(&key2, None).await?; // No callback
    assert_eq!(data2.len(), fetched_data2.len());
    // Optionally compare content if memory allows: assert_eq!(data2, fetched_data2);

    // 4. Listing Keys
    let keys = mutant.list_keys().await?;
    assert!(keys.contains(&key1));
    assert!(keys.contains(&key2));

    let key_details = mutant.list_key_details().await?;
    // You might want to iterate or check key_details here in a real application

    // 5. Getting Storage Stats
    let stats = mutant.get_storage_stats().await?;
    // Use stats as needed

    // 6. Updating Data
    let updated_data1 = b"Hello, Updated MutAnt World!".to_vec();
    mutant.update(key1.clone(), &updated_data1, None).await?;
    let fetched_updated_data1 = mutant.fetch(&key1, None).await?;
    assert_eq!(updated_data1, fetched_updated_data1);

    // 7. Removing Data
    mutant.remove(&key1).await?;

    // Verify removal
    match mutant.fetch(&key1, None).await {
        Err(Error::KeyNotFound(_)) => { /* Expected */ }
        Ok(_) => panic!("Key should have been removed!"),
        Err(e) => return Err(e.into()),
    }

    let keys_after_remove = mutant.list_keys().await?;
    assert!(!keys_after_remove.contains(&key1));
    assert!(keys_after_remove.contains(&key2));

    // Check stats again
    let stats_after_remove = mutant.get_storage_stats().await?;
    // Use stats_after_remove as needed
    // assert!(stats_after_remove.free_pads > stats.free_pads); // This depends on pad allocation details

    Ok(())
}

// Main entry point using tokio
fn main() {
    env_logger::init(); // Initialize logger

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        if let Err(e) = run_mutant_examples().await {
            eprintln!("\nError during example run: {}", e);
            // Print cause chain if available
            let mut source = e.source();
            while let Some(cause) = source {
                eprintln!("  Caused by: {}", cause);
                source = cause.source();
            }
            std::process::exit(1);
        }
    });
}

```

**Before Running:**

1.  Replace `"your twelve word mnemonic phrase here"` with your actual Autonomi wallet mnemonic.
2.  Replace `"your_autonomi_account_private_key_hex_string_here"` with the corresponding private key for that wallet.
3.  Ensure you have configured the `autonomi::Client` appropriately (e.g., network endpoints) if not using defaults.
4.  Add `mutant-lib`, `tokio`, `env_logger`, and `futures` (if not already present via `mutant-lib` re-exports) to your `Cargo.toml` dependencies. 