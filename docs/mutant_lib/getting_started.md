# MutAnt Library: Getting Started

This guide provides a basic example of how to use `mutant-lib` to store and retrieve data on the Autonomi network.

## 1. Prerequisites

*   **Rust Environment:** Ensure you have a working Rust development environment installed.
*   **Autonomi Account:** You need an Autonomi wallet (specifically, the private key) to interact with the network.
*   **`mutant-lib` Dependency:** Add `mutant-lib` to your `Cargo.toml`:

    ```toml
    [dependencies]
    mutant-lib = { version = "0.1" } # Use the appropriate version
    tokio = { version = "1", features = ["full"] }
    hex = "0.4"
    # Add other necessary dependencies
    ```

## 2. Basic Example

This example demonstrates initializing the library, storing a piece of data, fetching it back, and then removing it.

```rust
use mutant_lib::{
    api::MutAnt, // The main API entry point
    types::MutAntConfig, // Configuration options
    error::Error, // Library's error type
    events::{InitCallback, InitEvent, ProgressCallback, ProgressEvent}
};
use std::sync::{Arc, Mutex};

// --- Callback Setup (Optional, for progress reporting) ---

// Example Init Callback
fn my_init_callback(event: InitEvent) {
    match event {
        InitEvent::Starting => println!("Initialization starting..."),
        InitEvent::Connecting => println!("Connecting to network..."),
        InitEvent::LoadingIndex => println!("Loading master index..."),
        InitEvent::CreatingIndex => println!("Master index not found, creating new one..."),
        InitEvent::SavingIndex => println!("Saving master index..."),
        InitEvent::Finished(Ok(_)) => println!("Initialization successful!"),
        InitEvent::Finished(Err(e)) => eprintln!("Initialization failed: {}", e),
    }
}

// Example Progress Callback (for Store/Fetch)
fn my_progress_callback(event: ProgressEvent) {
    match event {
        ProgressEvent::Starting(op_type, total_bytes) => {
            println!("{} operation started for {} bytes", op_type, total_bytes);
        }
        ProgressEvent::Progress(op_type, bytes_processed, total_bytes) => {
            let percent = (bytes_processed as f64 / total_bytes as f64) * 100.0;
            println!("{} progress: {:.2}% ({}/{})", op_type, percent, bytes_processed, total_bytes);
        }
        ProgressEvent::Retrying(op_type, attempt, max_attempts, delay) => {
            println!("{} failed, retrying (attempt {}/{}) after {:?}...", op_type, attempt, max_attempts, delay);
        }
        ProgressEvent::Finished(op_type, Ok(_)) => {
            println!("{} operation successful!", op_type);
        }
        ProgressEvent::Finished(op_type, Err(e)) => {
            eprintln!("{} operation failed: {}", op_type, e);
        }
    }
}

// --- Main Application Logic ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Replace with your actual Autonomi private key hex string
    let private_key_hex = "YOUR_PRIVATE_KEY_HEX";

    // Basic configuration (can be customized)
    let config = MutAntConfig::default();

    // Wrap callbacks in Arc/Mutex if needed across threads, or just pass function pointers
    let init_cb: Option<InitCallback> = Some(Arc::new(Mutex::new(my_init_callback)));
    let progress_cb: Option<ProgressCallback> = Some(Arc::new(Mutex::new(my_progress_callback)));

    println!("Initializing MutAnt...");
    let mutant = match MutAnt::init_with_progress(private_key_hex.to_string(), config, init_cb).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to initialize MutAnt: {}", e);
            return Err(Box::new(e));
        }
    };
    println!("MutAnt initialized.");

    // -- Store Data --
    let key = "my_first_data".to_string();
    let data_to_store = b"Hello, MutAnt World!".to_vec();

    println!("Storing data under key: '{}'", key);
    match mutant.store_with_progress(&key, &data_to_store, progress_cb.clone()).await {
        Ok(_) => println!("Successfully stored data."),
        Err(e) => {
            eprintln!("Failed to store data: {}", e);
            // Decide how to handle the error (e.g., retry, exit)
        }
    }

    // -- Fetch Data --
    println!("Fetching data for key: '{}'", key);
    match mutant.fetch_with_progress(&key, progress_cb.clone()).await {
        Ok(retrieved_data) => {
            println!("Successfully fetched data.");
            assert_eq!(data_to_store, retrieved_data);
            println!("Retrieved data matches stored data!");
            // Convert to string for display (if applicable)
            if let Ok(s) = String::from_utf8(retrieved_data) {
                println!("Data content: '{}'", s);
            } else {
                println!("Data is not valid UTF-8");
            }
        }
        Err(Error::KeyNotFound(k)) => {
            eprintln!("Key '{}' not found on the network.", k);
        }
        Err(e) => {
            eprintln!("Failed to fetch data: {}", e);
        }
    }

    // -- Remove Data --
    println!("Removing data for key: '{}'", key);
    match mutant.remove(&key).await {
        Ok(_) => println!("Successfully removed data."),
        Err(e) => {
            eprintln!("Failed to remove data: {}", e);
        }
    }

    // -- Verify Removal (Optional) --
    println!("Verifying removal by fetching again...");
    match mutant.fetch(&key).await {
        Ok(_) => {
            eprintln!("Error: Data still exists after removal!");
        }
        Err(Error::KeyNotFound(k)) => {
            println!("Confirmed: Key '{}' not found after removal.", k);
        }
        Err(e) => {
            eprintln!("Failed to fetch data during verification: {}", e);
        }
    }

    println!("Example finished.");
    Ok(())
}

```

## 3. Running the Example

1.  Replace `"YOUR_PRIVATE_KEY_HEX"` with your actual Autonomi private key.
2.  Save the code as `main.rs` (or similar).
3.  Run `cargo run`.

You should see output indicating the progress of initialization, storing, fetching, and removing the data.

## 4. Next Steps

*   Explore `core_concepts.md` to understand the underlying mechanisms.
*   Check `api_reference.md` for details on all available methods and configuration options.
*   Review `best_practices.md` for tips on using the library effectively. 