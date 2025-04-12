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

// --- Callbacks (Optional) ---

// Example Init Callback
fn my_init_callback(event: InitProgressEvent) {
    match event {
        InitProgressEvent::Connecting => println!("Initializing: Connecting..."),
        InitProgressEvent::LoadingMasterIndex => println!("Initializing: Loading master index..."),
        InitProgressEvent::CreatingMasterIndex => println!("Initializing: Master index not found, creating new one..."),
        InitProgressEvent::SavingMasterIndex => println!("Initializing: Saving master index..."),
        InitProgressEvent::Ready => println!("Initializing: Ready!"),
    }
}

// Example Put Callback (using Arc for potential multi-threaded GUI updates)
fn my_put_callback(event: PutEvent, progress_state: Option<Arc<tokio::sync::Mutex<f32>>>) {
    match event {
        PutEvent::AllocatingPads { required, reused } => {
            println!("Storing: Allocating {} pads ({} reused from free list)...", required, reused);
        }
        PutEvent::WritingPad { current, total } => {
            let percentage = (current as f32 / total as f32) * 100.0;
            println!("Storing: Writing pad {} of {} ({:.1}%)", current, total, percentage);
            if let Some(state) = progress_state {
                let state_clone = state.clone();
                tokio::spawn(async move {
                    let mut locked_state = state_clone.lock().await;
                    *locked_state = percentage;
                });
            }
        }
        PutEvent::UpdatingMasterIndex => println!("Storing: Updating master index..."),
        PutEvent::SavingMasterIndex => println!("Storing: Saving master index..."),
        PutEvent::Complete => println!("Storing: Complete!"),
    }
}

// Example Get Callback
fn my_get_callback(event: GetEvent) {
    match event {
        GetEvent::FetchingPadInfo => println!("Fetching: Looking up key info..."),
        GetEvent::FetchingPadData { current, total, bytes_downloaded, total_bytes } => {
            let pad_percentage = (current as f32 / total as f32) * 100.0;
            let byte_percentage = (bytes_downloaded as f32 / total_bytes as f32) * 100.0;
            println!(
                "Fetching: Pad {} of {} ({:.1}% done), Total Progress: {:.1}% ({}/{})",
                current, total, pad_percentage, byte_percentage, bytes_downloaded, total_bytes
            );
        }
        GetEvent::ReassemblingData => println!("Fetching: Reassembling data chunks..."),
        GetEvent::Complete => println!("Fetching: Complete!"),
    }
}

// --- Main Async Function ---

async fn run_mutant_examples() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialization
    println!("--- 1. Initializing MutAnt ---");
    let wallet = Wallet::from_phrase("your twelve word mnemonic phrase here", None)?;
    let private_key_hex = "your_autonomi_account_private_key_hex_string_here".to_string(); // Ensure this matches the wallet

    // Initialize with callback
    let (mutant, _init_handle) = MutAnt::init(
        wallet.clone(),
        private_key_hex.clone(),
        Some(Box::new(my_init_callback)),
    )
    .await?;

    // Wait a moment for potential background init tasks
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 2. Storing Data
    println!("\n--- 2. Storing Data ---");
    let key1 = "hello.txt".to_string();
    let data1 = b"Hello, MutAnt World!".to_vec();

    // Simple store
    println!("Storing '{}' ({} bytes)...", key1, data1.len());
    mutant.store(key1.clone(), &data1, None).await?;
    println!("Stored '{}' successfully.", key1);

    // Store larger data with progress callback
    let key2 = "large_data.bin".to_string();
    // Example: create ~10MB of data (adjust size as needed)
    let data2: Vec<u8> = (0..(10 * 1024 * 1024)).map(|i| (i % 256) as u8).collect();

    println!("\nStoring '{}' ({} bytes) with progress...", key2, data2.len());
    let progress_state = Arc::new(tokio::sync::Mutex::new(0.0f32));
    let progress_state_clone = progress_state.clone();
    let put_cb = Box::new(move |event| my_put_callback(event, Some(progress_state_clone.clone())));

    mutant.store(key2.clone(), &data2, Some(put_cb)).await?;
    println!("Stored '{}' successfully.", key2);

    // 3. Fetching Data
    println!("\n--- 3. Fetching Data ---");

    // Fetch small data
    println!("Fetching '{}'...", key1);
    let fetched_data1 = mutant.fetch(&key1, None).await?;
    println!("Fetched '{}': Size {}, Content: '{}'", key1, fetched_data1.len(), String::from_utf8_lossy(&fetched_data1));
    assert_eq!(data1, fetched_data1);

    // Fetch large data with progress
    println!("\nFetching '{}' with progress...", key2);
    let get_cb = Box::new(my_get_callback);
    let fetched_data2 = mutant.fetch(&key2, Some(get_cb)).await?;
    println!("Fetched '{}': Size {}", key2, fetched_data2.len());
    assert_eq!(data2.len(), fetched_data2.len());
    // Optionally compare content if memory allows: assert_eq!(data2, fetched_data2);

    // 4. Listing Keys
    println!("\n--- 4. Listing Keys ---");
    let keys = mutant.list_keys().await?;
    println!("Found keys: {:?}", keys);
    assert!(keys.contains(&key1));
    assert!(keys.contains(&key2));

    let key_details = mutant.list_key_details().await?;
    println!("Key details:");
    for detail in key_details {
        println!("  - Key: {}, Size: {}, Modified: {}", detail.key, detail.size, detail.modified);
    }

    // 5. Getting Storage Stats
    println!("\n--- 5. Getting Storage Stats ---");
    let stats = mutant.get_storage_stats().await?;
    println!("Storage Stats: {:#?}", stats);

    // 6. Updating Data
    println!("\n--- 6. Updating Data ---");
    let updated_data1 = b"Hello, Updated MutAnt World!".to_vec();
    println!("Updating '{}'...", key1);
    mutant.update(key1.clone(), &updated_data1, None).await?;
    let fetched_updated_data1 = mutant.fetch(&key1, None).await?;
    println!("Fetched updated '{}': '{}'", key1, String::from_utf8_lossy(&fetched_updated_data1));
    assert_eq!(updated_data1, fetched_updated_data1);

    // 7. Removing Data
    println!("\n--- 7. Removing Data ---");
    println!("Removing '{}'...", key1);
    mutant.remove(&key1).await?;
    println!("Removed '{}'.", key1);

    // Verify removal
    match mutant.fetch(&key1, None).await {
        Err(Error::KeyNotFound(_)) => println!("Verified: Key '{}' not found after removal.", key1),
        Ok(_) => panic!("Key should have been removed!"),
        Err(e) => return Err(e.into()),
    }

    let keys_after_remove = mutant.list_keys().await?;
    println!("Keys after removal: {:?}", keys_after_remove);
    assert!(!keys_after_remove.contains(&key1));
    assert!(keys_after_remove.contains(&key2));

    // Check stats again - free pads should increase if key1 used unique pads
    println!("\nGetting stats after removal...");
    let stats_after_remove = mutant.get_storage_stats().await?;
    println!("Stats after remove: {:#?}", stats_after_remove);
    // assert!(stats_after_remove.free_pads > stats.free_pads); // This depends on pad allocation details

    println!("\n--- Example Run Complete ---");
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