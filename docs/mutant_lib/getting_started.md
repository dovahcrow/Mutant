# MutAnt Library: Getting Started

This guide provides a basic example of how to use `mutant-lib` to store and retrieve data on the Autonomi network.

## 1. Prerequisites

* **Rust Environment:** Ensure you have a working Rust development environment installed.
* **Autonomi Account:** You need an Autonomi wallet (specifically, the private key) to interact with the network.
* **`mutant-lib` Dependency:** Add `mutant-lib` to your `Cargo.toml`:

```toml
[dependencies]
mutant-lib = "0.6.1"
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

## 2. Basic Example

This example demonstrates initializing the library, storing a piece of data, fetching it back, and then removing it.

```rust
use mutant_lib::{MutAnt, storage::StorageMode, error::Error};
use std::sync::Arc;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Replace with your actual Autonomi private key hex string
    let private_key_hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    println!("Initializing MutAnt...");
    let mutant = MutAnt::init(private_key_hex).await?;
    println!("MutAnt initialized.");

    // -- Store Data --
    let key = "my_first_data";
    let data_to_store = b"Hello, MutAnt World!";

    println!("Storing data under key: '{}'", key);
    let address = mutant.put(
        key,
        Arc::new(data_to_store.to_vec()),
        StorageMode::Medium,
        false, // not public
        false, // verify
        None,  // no callback
    ).await?;
    println!("Successfully stored data at address: {}", address);

    // -- Fetch Data --
    println!("Fetching data for key: '{}'", key);
    let retrieved_data = mutant.get(key, None).await?;
    println!("Successfully fetched data.");
    assert_eq!(data_to_store.to_vec(), retrieved_data);
    println!("Retrieved data matches stored data!");

    // Convert to string for display
    let data_string = String::from_utf8_lossy(&retrieved_data);
    println!("Data content: '{}'", data_string);

    // -- Remove Data --
    println!("Removing data for key: '{}'", key);
    mutant.rm(key).await?;
    println!("Successfully removed data.");

    // -- Verify Removal (Optional) --
    println!("Verifying removal by fetching again...");
    match mutant.get(key, None).await {
        Ok(_) => {
            eprintln!("Error: Data still exists after removal!");
        }
        Err(e) => {
            println!("Confirmed: Key '{}' not found after removal: {}", key, e);
        }
    }

    println!("Example finished.");
    Ok(())
}
```

## 3. Public Data Example

This example demonstrates storing and retrieving public data:

```rust
use mutant_lib::{MutAnt, storage::StorageMode, storage::ScratchpadAddress};
use std::sync::Arc;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize with a private key
    let private_key_hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let mutant = MutAnt::init(private_key_hex).await?;

    // Store data publicly
    let key = "public_data";
    let data = b"This data is publicly accessible";

    let address = mutant.put(
        key,
        Arc::new(data.to_vec()),
        StorageMode::Medium,
        true,  // public flag set to true
        false, // verify
        None,  // no callback
    ).await?;

    // Get the public index address
    let public_address = mutant.get_public_index_address(key).await?;
    println!("Data stored publicly at address: {}", public_address);

    // Now anyone can fetch this data without the private key
    let public_fetcher = MutAnt::init_public().await?;

    // Convert the hex address to a ScratchpadAddress
    let address = ScratchpadAddress::from_hex(&public_address)?;

    // Fetch the public data
    let retrieved_data = public_fetcher.get_public(&address, None).await?;

    // Verify the data
    assert_eq!(data.to_vec(), retrieved_data);
    println!("Successfully retrieved public data!");

    Ok(())
}
```

## 4. Using Callbacks

This example demonstrates using callbacks to track progress:

```rust
use mutant_lib::{MutAnt, storage::StorageMode, events::{PutCallback, PutEvent}};
use std::sync::Arc;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let private_key_hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let mutant = MutAnt::init(private_key_hex).await?;

    // Create a callback function
    let callback = |event: PutEvent| {
        match event {
            PutEvent::PadReserved(pad_info) => {
                println!("Pad reserved: {}", pad_info.address);
            }
            PutEvent::PadWritten(pad_info) => {
                println!("Pad written: {}", pad_info.address);
            }
            PutEvent::PadConfirmed(pad_info) => {
                println!("Pad confirmed: {}", pad_info.address);
            }
            PutEvent::Complete => {
                println!("Operation complete!");
            }
        }
    };

    // Create a large data set to see progress
    let data = vec![0u8; 10 * 1024 * 1024]; // 10MB

    // Store with callback
    mutant.put(
        "large_file",
        Arc::new(data),
        StorageMode::Medium,
        false,
        false,
        Some(Box::new(callback)),
    ).await?;

    Ok(())
}
```

## 5. Running the Examples

1. Replace `"000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"` with your actual Autonomi private key or use this test key for local development.
2. Save the code as `main.rs` (or similar).
3. Run `cargo run`.

You should see output indicating the progress of initialization, storing, fetching, and removing the data.

## 6. Using the Daemon Architecture

For most applications, it's recommended to use the daemon architecture:

```rust
use mutant_client::MutantClient;
use mutant_protocol::StorageMode;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Connect to the daemon (must be running)
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3001/ws").await?;

    // Start a put operation in the background
    let (start_task, progress_rx) = client.put(
        "my_key",
        "path/to/file.txt",
        StorageMode::Medium,
        false, // not public
        false, // verify
    ).await?;

    // Monitor progress (optional)
    tokio::spawn(async move {
        while let Ok(progress) = progress_rx.recv().await {
            println!("Progress: {:?}", progress);
        }
    });

    // Wait for the task to complete
    let result = start_task.await?;
    println!("Task completed: {:?}", result);

    Ok(())
}
```

## 7. Next Steps

* Explore `core_concepts.md` to understand the underlying mechanisms
* Check `architecture.md` for details on the component layout
* Review the internals documentation for deep dives into specific components
