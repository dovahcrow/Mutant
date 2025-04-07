# Anthill: Private Mutable Key-Value Storage for Autonomi

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE) <!-- Add license badge if applicable -->
[![Crates.io](https://img.shields.io/crates/v/anthill_lib.svg)](https://crates.io/crates/anthill_lib) <!-- Add crates.io badge if published -->

**Anthill** provides a robust and asynchronous private mutable key-value storage layer built upon the Autonomi network's `Scratchpad` primitives. It simplifies interaction with the underlying network storage.

## Core Concepts

*   **Private Mutable Key-Value Storage:** Offers a clean, asynchronous key-value interface (`store`, `fetch`, `remove`) operating on byte arrays (`Vec<u8>`).
*   **Autonomi Integration:** Seamlessly utilizes Autonomi `Scratchpad` for data persistence, managed via the `autonomi` library's `Network` and `Wallet` components.
*   **Callback-based Progress:** Provides hooks for asynchronous progress reporting during potentially long-running operations like `store` and `fetch`.

## Key Features

*   **Simplified Key-Value API:** Abstracts the complexities of direct scratchpad management.
*   **Secure Wallet Integration:** Uses `autonomi::Wallet` derived from a user-provided private key for network interactions.
*   **User-Friendly Keys:** Operates on human-readable string keys.
*   **Asynchronous Design:** Built with `async`/`await` and `tokio` for non-blocking network operations.
*   **Progress Reporting:** Includes callbacks for monitoring `store` and `fetch` operations (e.g., reservation, upload/download progress).
*   **CLI Tool:** Includes the `anthill` command-line tool for easy interaction with the storage layer, including progress bars.

## Getting Started

### Prerequisites

*   Rust Toolchain (latest stable recommended)

### Command-Line Interface (CLI)

Anthill includes the `anthill` command for convenient command-line access. You'll need a wallet file (e.g., `anthill_wallet.json`) containing your private key as a JSON string. By default, `anthill` looks for `anthill_wallet.json` in the current directory.

**CLI Usage Examples:**

Assuming `anthill` is in your `PATH` or you are running from `target/release/`:

```bash
# Store a value directly 
anthill put mykey "my value"

# Get a value and print to stdout
anthill get mykey
# Output: my value

# Store a value from stdin (e.g., piping a file)
cat data.txt | anthill put filekey

# Force overwrite an existing key
echo "new content" | anthill put filekey -f

# List stored keys and basic usage summary
anthill ls
# Output:
# Usage: 10.00 KB / 4.00 MB (0.2%) - 1 scratchpads managed
# Stored keys:
# - mykey
# - filekey

# Remove a key
anthill rm mykey
```

### Library Usage

Add `anthill_lib` and its dependencies to your `Cargo.toml`:

**Library Example:**

This example demonstrates initializing the necessary components and performing basic store/fetch operations. It assumes you have a wallet file (`my_wallet.json`) with a private key.

```rust
use anthill_lib::{anthill::Anthill, storage::Storage, error::Error, events::{PutEvent, GetEvent}};
use autonomi::{Network, Wallet};
use std::{fs, path::PathBuf, sync::Arc};
use serde_json;
use futures::future::FutureExt; // Required for .boxed() on callbacks

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Wallet Setup ---
    let wallet_path = PathBuf::from("my_wallet.json"); // Replace with your wallet path
    let private_key_hex_json = fs::read_to_string(&wallet_path)?;
    let private_key_hex: String = serde_json::from_str(&private_key_hex_json)?;

    // --- Network & Wallet Initialization ---
    // Ensure you have a valid client.config for the network
    let network = Network::new(true).await?; // Use true for testnet/local
    let wallet = Wallet::new_from_private_key(network, &private_key_hex)?;

    // --- Storage Initialization ---
    // Storage::new now returns (Arc<Storage>, JoinHandle, InitialMapInfo, Arc<Mutex<MapInfo>>)
    // We provide None for the InitCallback for simplicity here.
    let (storage_arc, storage_init_handle, initial_map_info, map_info_mutex) =
        Storage::new(wallet, &private_key_hex, None).await?;

    // --- Anthill Initialization ---
    // We provide None for the default Vault Key callback.
    let anthill = Anthill::new(storage_arc.clone(), map_info_mutex, initial_map_info, None).await?;

    // --- Using Anthill ---
    let key = "my_library_key";
    let value_to_store = b"some data from the library".to_vec();

    println!("Storing value for key: {}", key);
    // Store requires a callback (PutCallback)
    let put_callback = Box::new(|event: PutEvent| async move {
        match event {
            PutEvent::StoreComplete => println!("Put operation complete."),
            _ => {} // Handle other events like progress if needed
        }
        Ok(true) // Return true to continue, false to cancel
    }.boxed());
    anthill.store(key.to_string(), &value_to_store, Some(put_callback)).await?;
    println!("Stored successfully call returned.");

    println!("Fetching value for key: {}", key);
    // Fetch requires a callback (GetCallback)
    let get_callback = Box::new(|event: GetEvent| async move {
         match event {
            GetEvent::DownloadFinished => println!("Get operation complete."),
             _ => {} // Handle other events like progress if needed
         }
        Ok(())
    }.boxed());
    let fetched_value = anthill.fetch(key, Some(get_callback)).await?;
    println!("Fetched successfully call returned.");

    assert_eq!(value_to_store, fetched_value);
    println!("Fetched value matches stored value.");

    println!("Removing key: {}", key);
    anthill.remove(key).await?;
    println!("Removed successfully.");

    // Verify removal (fetch requires a callback, even if just checking for KeyNotFound)
    let verify_callback = Box::new(|_event: GetEvent| async { Ok(()) }.boxed());
    match anthill.fetch(key, Some(verify_callback)).await {
        Err(Error::KeyNotFound(_)) => {
            println!("Verification successful: Key '{}' correctly reported as not found.", key);
        }
        Ok(_) => panic!("Error: Key should have been removed!"),
        Err(e) => panic!("Unexpected error during verification: {:?}", e),
    }

    // --- Shutdown ---
    // It's good practice to wait for background tasks if they were spawned.
    // The storage_init_handle completes when the initial storage setup is done.
    println!("Waiting for storage initialization task...");
    if let Err(e) = storage_init_handle.await {
         // Cancellation is expected if the main task finishes quickly.
         if !e.is_cancelled() {
             eprintln!("Background storage task join error: {}", e);
         }
    }
    println!("Library example finished.");

    Ok(())
}
```

## Development and Testing

Anthill relies on a running Autonomi network. For local development and running integration tests, scripts are provided to manage a self-contained test network.

### Local Testnet Management (`scripts/manage_local_testnet.sh`)

This script handles the setup, startup, and shutdown of a local Autonomi EVM testnet and the required `antnode` processes.

**Prerequisites:**

*   Ensure you have the necessary build tools for Rust (`cargo`) and Git.

**Setup:**

Before starting the testnet for the first time, run the setup command to clone the required Autonomi dependency:

```bash
./scripts/manage_local_testnet.sh setup
```

**Commands:**

*   `./scripts/manage_local_testnet.sh start`: Starts the local EVM and `antnode` network. Builds dependencies if needed.
*   `./scripts/manage_local_testnet.sh stop`: Stops the local EVM and `antnode` network processes.
*   `./scripts/manage_local_testnet.sh restart`: Stops and then starts the network.
*   `./scripts/manage_local_testnet.sh status`: Checks the running status of the EVM and `antnode` processes.
*   `./scripts/manage_local_testnet.sh logs`: Tails the log file for the local EVM testnet.
*   `./scripts/manage_local_testnet.sh clean`: Stops the network and removes all associated network data stored in `test_network_data`.

**Important Environment Variable (`XDG_DATA_HOME`)**

The management script configures the testnet to store its data within the `./test_network_data` directory in your project root by setting the `XDG_DATA_HOME` environment variable *within the script's context*.

When interacting with this script-managed testnet using commands *outside* the script (e.g., running `cargo run -- ...` or the `anthill` binary directly), you **MUST** set the `XDG_DATA_HOME` environment variable manually in your shell to match the script's location, otherwise the client will not find the network configuration:

```bash
# Make sure the testnet is running via the script first
./scripts/manage_local_testnet.sh start

# Set the variable for your current shell session
export XDG_DATA_HOME="$(pwd)/test_network_data"

# Now run your cargo command or the anthill binary
# (Assuming anthill is built and in your PATH or target/release)
anthill ls

# Alternatively, use a tool like direnv to set it automatically
# Add 'export XDG_DATA_HOME="${PWD}/test_network_data"' to a .envrc file
```

### Running Integration Tests (`scripts/run_tests_with_env.sh`)

This script automates the process of running the integration tests (located in the `tests` directory, e.g., `tests/anthill_test.rs`):

1.  Ensures dependencies are set up using `manage_local_testnet.sh setup`.
2.  Starts the local testnet using `manage_local_testnet.sh start`.
3.  Sets the necessary `XDG_DATA_HOME` environment variable for the tests.
4.  Runs `cargo test` targeting the integration tests.
5.  Automatically cleans up and stops the testnet afterwards using `manage_local_testnet.sh clean` (even if tests fail).

**Usage:**

```bash
./scripts/run_tests_with_env.sh
```

## Architecture Overview

Anthill leverages several components from the `autonomi` crate and its own library (`anthill_lib`):

1.  **`autonomi::Network`**: Handles connection and interaction with the Autonomi network based on `client.config`.
2.  **`autonomi::Wallet`**: Manages user identity and signing capabilities derived from a private key.
3.  **`anthill_lib::storage::Storage`**: The foundational layer interacting directly with the Autonomi network via `autonomi::Client` (obtained from `Network` and `Wallet`). It handles:
    *   Low-level scratchpad operations (creation, updates, fetches).
    *   Interaction with the Autonomi `Vault` for managing keys associated with scratchpads (details might be internal to `autonomi`).
    *   Maintains crucial internal state like the `MapInfo` which tracks scratchpad allocations and usage. It runs background tasks for initialization.
    *   Provides `InitCallback` for monitoring initialization progress.
4.  **`anthill_lib::anthill::Anthill`**: The high-level abstraction layer providing the primary key-value API (`store`, `fetch`, `remove`, `get_summary`, `get_user_keys`). It:
    *   Coordinates operations using the underlying `Storage` layer and shared `MapInfo`.
    *   Handles user-facing key management.
    *   Manages `PutCallback` and `GetCallback` for progress reporting during `store` and `fetch` operations.
    *   Provides summary information about storage usage.

Data is stored and retrieved as raw byte vectors (`Vec<u8>`), allowing the user flexibility in serialization/deserialization.

## Configuration

The primary configuration is the wallet file path (via `--wallet-path` CLI argument or the default `anthill_wallet.json`) and the Autonomi network configuration (`client.config` discovered via `XDG_DATA_HOME` or default locations). Internal constants within `anthill_lib` define behaviors like scratchpad size.

## License

*(Specify the license for this project here, e.g., MIT or Apache-2.0)*

## Contributing

*(Add guidelines for contributing to this project here)* 
