# MutAnt: Private Mutable Key-Value Storage for Autonomi

[![Build Status](https://github.com/Champii/MutAnt/actions/workflows/rust.yml/badge.svg)](https://github.com/Champii/MutAnt/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/mutant_lib.svg)](https://crates.io/crates/mutant_lib)

**MutAnt** provides a robust and asynchronous private mutable key-value storage layer built upon the Autonomi network's `Scratchpad` primitives. It simplifies interaction with the underlying network storage.

> **⚠️ Disclaimer:** MutAnt is currently under active development and is **not ready for production use or mainnet deployment**. Use it at your own risk. Expect potential bugs, breaking changes, and incomplete features. Check how to spin a local testnet [here](#local-testnet-management-scriptsmanage_local_testnetsh).

## Core Concepts

*   **Private Mutable Key-Value Storage:** Offers a clean, asynchronous key-value interface (`get`, `put`, `update` `rm`) operating on byte arrays
*   **User-Friendly Keys:** Operates on human-readable string keys.
*   **Asynchronous Design:** Built with `async`/`await` and `tokio` for non-blocking network operations.
*   **Progress Reporting:** Includes callbacks for monitoring `store` and `fetch` operations (e.g., reservation, upload/download progress).
*   **CLI Tool:** Includes the `mutant` command-line tool for easy interaction with the storage layer, including progress bars.

## Getting Started

### Prerequisites

*   Rust Toolchain (latest stable recommended)

### Installation

#### Install from crates.io (Recommended)
```bash

cargo install mutant
```

#### Local installation for development

```bash
git clone https://github.com/Champii/MutAnt.git
cd MutAnt
cargo install --path .
```

### Command-Line Interface (CLI)

MutAnt includes the `mutant` command for convenient command-line access.

**CLI Usage Examples:**

Assuming `mutant` is in your `PATH` or you are running from `target/release/`:

```
Mutant CLI - Interact with the Mutant network

Usage: mutant-cli [OPTIONS] <COMMAND>

Commands:
  put    Puts a key-value pair onto the network. Reads value from stdin if omitted. Use --force to overwrite an existing key
  get    Gets the value for a given key from the network and prints it to stdout
  rm     Deletes a key-value pair from the network
  ls     Lists all keys stored on the network
  stats  Get storage summary (allocator perspective)
  reset  Resets the master index to its initial empty state. Requires confirmation
  help   Print this message or the help of the given subcommand(s)

Options:
  -l, --local    Path to the wallet file (JSON containing private key string) Use local network (Devnet) instead of Mainnet
  -h, --help     Print help
  -V, --version  Print version
```

```bash
# Store a value directly 
mutant put mykey "my value"

# Get a value and print to stdout
mutant get mykey
# Output: my value

# Store a value from stdin (e.g., piping a file)
cat data.txt | mutant put mykey2

# Force overwrite an existing key
echo "new content" | mutant put mykey2 -f

# List stored keys and basic usage summary
mutant ls
#mykey
#mykey2

# Remove a key
mutant rm mykey
```

### Library Usage

Add `mutant_lib` and its dependencies to your `Cargo.toml`:

**Library Example:**

This example demonstrates initializing the necessary components and performing basic store/fetch operations. It assumes you have a wallet file (`my_wallet.json`) with a private key.

```rust
use mutant_lib::{mutant::MutAnt, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let private_key_hex = '0xYOUR_PRIVATE_KEY_HERE';

    let (mutant, _init_handle) = MutAnt::init(private_key_hex).await?;

    mutant.store("hello", b"world").await?;

    let fetched_value = mutant.fetch("hello").await?;

    println!("Fetched value: {:?}", fetched_value);

    mutant.remove("hello").await?;

    Ok(())
}
```

## Development and Testing

MutAnt relies on a running Autonomi network. For local development and running integration tests, scripts are provided to manage a self-contained test network.

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

When interacting with this script-managed testnet using commands *outside* the script (e.g., running `cargo run -- ...` or the `mutant` binary directly), you **MUST** set the `XDG_DATA_HOME` environment variable manually in your shell to match the script's location, otherwise the client will not find the network configuration:

```bash
# Make sure the testnet is running via the script first
./scripts/manage_local_testnet.sh start

# Set the variable for your current shell session
export XDG_DATA_HOME="$(pwd)/test_network_data"

# Now run your cargo command or the mutant binary
cargo run -- --local ls
```

### Running Integration Tests (`scripts/run_tests_with_env.sh`)

This script automates the process of running the integration tests (located in the `tests` directory, e.g., `tests/mutant_test.rs`):

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

MutAnt leverages several components from the `autonomi` crate and its own library (`mutant_lib`):

1.  **`autonomi::Network`**: Handles connection and interaction with the Autonomi network based on `client.config`.
2.  **`autonomi::Wallet`**: Manages user identity and signing capabilities derived from a private key.
3.  **`mutant_lib::storage::Storage`**: The foundational layer interacting directly with the Autonomi network via `autonomi::Client` (obtained from `Network` and `Wallet`). It handles:
    *   Low-level scratchpad operations (creation, updates, fetches).
    *   Interaction with the Autonomi `Vault` for managing keys associated with scratchpads (details might be internal to `autonomi`).
    *   Maintains crucial internal state like the `MapInfo` which tracks scratchpad allocations and usage. It runs background tasks for initialization.
    *   Provides `InitCallback` for monitoring initialization progress.
4.  **`mutant_lib::mutant::MutAnt`**: The high-level abstraction layer providing the primary key-value API (`store`, `fetch`, `remove`, `get_summary`, `get_user_keys`). It:
    *   Coordinates operations using the underlying `Storage` layer and shared `MapInfo`.
    *   Handles user-facing key management.
    *   Manages `PutCallback` and `GetCallback` for progress reporting during `store` and `fetch` operations.
    *   Provides summary information about storage usage.

Data is stored and retrieved as raw byte vectors (`Vec<u8>`), allowing the user flexibility in serialization/deserialization.

## Configuration

The primary configuration is the wallet file path (via `--wallet-path` CLI argument or the default `mutant_wallet.json`) and the Autonomi network configuration (`client.config` discovered via `XDG_DATA_HOME` or default locations). Internal constants within `mutant_lib` define behaviors like scratchpad size.

## License

*(Specify the license for this project here, e.g., MIT or Apache-2.0)*

## Contributing

*(Add guidelines for contributing to this project here)* 
