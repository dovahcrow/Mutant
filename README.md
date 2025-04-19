# MutAnt: Mutable Key-Value Storage for Autonomi

<!-- Badges -->
[![Build Status](https://github.com/Champii/MutAnt/actions/workflows/rust.yml/badge.svg)](https://github.com/Champii/MutAnt/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/mutant-lib.svg)](https://crates.io/crates/mutant-lib)
[![Docs.rs](https://docs.rs/mutant_lib/badge.svg)](https://docs.rs/mutant_lib)
[![License: LGPLv3](https://img.shields.io/badge/license-LGPLv3-blue.svg)](LICENSE)

> **MutAnt** is a public/private mutable key-value store on the Autonomi network, featuring resumable uploads, local index caching, and a powerful async Rust API + CLI.

**Note:** No LLM was harmed in the making of this project.

## Table of Contents
1. [Core Concepts](#core-concepts)
2. [Getting Started](#getting-started)
3. [Installation](#installation)
4. [Command-Line Interface (CLI)](#command-line-interface-cli)
5. [Library Usage](#library-usage)
6. [Development and Testing](#development-and-testing)
7. [Architecture Overview](#architecture-overview)
8. [Configuration](#configuration)
9. [License](#license)
10. [Contributing](#contributing)

## Core Concepts

*   **Mutable Key-Value Storage:** Offers a clean, asynchronous key-value interface (`get`, `put`, `rm`).
*   **Public/Private Uploads:** Store data publicly to share with others (no encryption) or store privately (encrypted with your private key).
*   **Resumable Uploads:** Automatic resume of interrupted uploads; pick up right where you left off.
*   **Efficient Space Reuse:** Frees and reuses storage pads, minimizing on-chain costs.
*   **Local Cache Index:** Fast local lookups and seamless remote synchronization.
*   **Async-first Design:** Built on `tokio` for high-performance non-blocking operations.
*   **Dual Interface:** Use as a Rust library (`mutant-lib`) or via the `mutant` CLI.

## Getting Started

### Prerequisites

*   Rust Toolchain (latest stable recommended)
*   `ant` CLI configured with a wallet (see below)

### Setup `ant` Wallet

#### If you just want to fetch public data, you can skip this section.

Before using `mutant`, you need to have an `ant` wallet configured for the target network (Mainnet by default, or Devnet if using the `--local` flag). If you don't have `ant` installed, you can get it using [antup](https://github.com/maidsafe/antup):

```bash
antup client
```

Once `ant` is installed, if you haven't already, you can import your existing Ethereum/ANT wallet's private key using the `ant` CLI:

```bash
ant wallet import YOUR_PRIVATE_KEY_HERE
```

Replace `YOUR_PRIVATE_KEY_HERE` with your actual private key. `mutant` will automatically detect and use this wallet.

Alternatively, you can create a new empty wallet using `ant wallet create` and fund it with the necessary ANT or ETH later.

MutAnt will look for your ant wallets and ask you which one you want to use if you have multiple on the first run, then save your choice in `~/.config/mutant/config.json`.

## Installation

```bash
cargo install mutant
```

## Command-Line Interface (CLI)

MutAnt includes the `mutant` command for convenient command-line access.

**CLI Usage Examples:**

```bash
mutant --help
```
```text
Distributed mutable key value storage over the Autonomi network

Usage: mutant [OPTIONS] <COMMAND>

Commands:
  put      Stores private or public data. Use `-p <name>` for public uploads (no encryption). Use `--force` to overwrite. Reads value from stdin if omitted
  get      Gets the value for a given key from the network and prints it to stdout
  rm       Deletes a key-value pair from the network
  ls       Lists all stored private keys and public uploads. Use -l for detailed view including public addresses
  stats    Get storage summary (allocator perspective)
  reset    Resets the master index to its initial empty state. Requires confirmation
  import   Imports a free scratchpad using its private key
  sync     Synchronize local index cache with remote storage. Use --push-force to overwrite remote index
  purge    Deletes all data associated with the current wallet. Requires confirmation
  reserve  Pre-allocates a number of empty scratchpads for future use
  help     Print this message or the help of the given subcommand(s)

Options:
  -l, --local    Use local network (Devnet) instead of Mainnet
  -q, --quiet    Suppress informational output (logs, progress bars)
  -h, --help     Print help
  -V, --version  Print version
```

**Basic Usage:**

Store/fetch private data:

```bash
# Store a value directly
mutant put mykey "my value"

# Get a value and print to stdout
mutant get mykey
# Output: my value

# Update a value (you can use the shorter -f)
mutant put mykey "my new value" --force

# Remove a value
mutant rm mykey
```

Store/fetch public data:

```bash
# Store data publicly (no encryption) under a name
mutant put -p my_key "some public content"
# Output: 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef

# Get your own public data by name
mutant get my_key
# Output: some public content

# Get public data by address
mutant get -p 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
# Output: some public content

# You can update it all the same as the private data
mutant put -p my_key "some updated public content" --force
```

Pipes and redirects:

```bash
# Store a value from stdin (e.g., piping a file)
cat data.txt | mutant put mykey2

mutant get mykey2 > fetched_data.txt
```

Stats and debug:


```bash
# List stored keys
mutant ls
# mykey
# mykey2
# public_data @ 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef

# List keys with details (size, last modified)
mutant ls -l
# SIZE TYPE    MODIFIED     KEY/NAME
# 3 B  Private Apr 19 00:51 mykey
# 5 B  Private Apr 19 00:51 mykey2
# 11 B Public  Apr 19 00:51 public_data @ 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef

# Sync local index with remote storage
mutant sync

# Pre-allocate 5 scratchpads
mutant reserve 5

# View storage statistics
mutant stats
```

### Screenshots

```bash
$> cat big_file | mutant put my_big_file
```

![Put Progress Screenshot](docs/screenshots/put_screenshot1.png)

```bash
$> mutant stats
```

![Stats Screenshot](docs/screenshots/stats_screenshot1.png)

## Library Usage

Add `mutant-lib` and its dependencies to your `Cargo.toml`:

```toml
[dependencies]
mutant-lib = "0.3" # Or the version you need
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

**Library Example:**

This example demonstrates initializing the necessary components and performing basic store/fetch operations. It assumes you have an `ant` wallet setup.

```rust
use mutant_lib::{MutAnt, MutAntConfig, Error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Replace with your actual private key (hex format, with or without 0x prefix)
    let private_key_hex = "0xYOUR_PRIVATE_KEY_HEX".to_string();

    let mut mutant = MutAnt::init(private_key_hex).await?;

    mutant.store("greeting", b"hello world").await?;

    let fetched_value = mutant.fetch("greeting").await?;

    println!("Fetched value: {}", String::from_utf8_lossy(&fetched_value));

    mutant.remove("greeting").await?;

    Ok(())
}

### Fetching Public Data (Keyless Initialization)

If your application only needs to retrieve publicly stored data (using `store_public`) and doesn't need to manage private data, you can initialize a lightweight `MutAnt` instance without a private key using `MutAnt::init_public()`:

```rust
use mutant_lib::{MutAnt, Error};
use mutant_lib::storage::ScratchpadAddress;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize a public fetcher instance (defaults to Mainnet)
    let public_fetcher = MutAnt::init_public().await?;

    // Assume you have the public address from elsewhere
    let address_hex = "..."; // Replace with actual public address hex
    let public_address = ScratchpadAddress::from_hex(address_hex)?;

    // Fetch the public data
    match public_fetcher.fetch_public(public_address, None).await {
        Ok(data) => println!("Fetched public data: {} bytes", data.len()),
        Err(e) => eprintln!("Failed to fetch public data: {}", e),
    }

    Ok(())
}

This keyless instance is optimized for fetching public data and cannot perform operations requiring a private key (like `store`, `fetch` private, `remove`).
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

When interacting with this script-managed testnet using commands *outside* the script (e.g., running `cargo run --package mutant -- ...` or the `mutant` binary directly), you **MUST** set the `XDG_DATA_HOME` environment variable manually in your shell to match the script's location, otherwise the client will not find the network configuration:

```bash
# Make sure the testnet is running via the script first
./scripts/manage_local_testnet.sh start

# Set the variable for your current shell session
export XDG_DATA_HOME="$(pwd)/test_network_data"

# Now run your cargo command or the mutant binary
cargo run --package mutant -- --local ls
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

MutAnt is structured as a Rust workspace with two main crates:

1.  **`mutant-lib`**: The core library containing all the logic for interacting with the Autonomi network, managing storage, handling data chunking, indexing, and providing the asynchronous API.
2.  **`mutant-cli`**: A command-line interface built on top of `mutant-lib`, providing user-friendly access to the storage features.

The library (`mutant-lib`) leverages several components:

1.  **`autonomi::Network`**: Handles connection and interaction with the Autonomi network based on `client.config`.
2.  **`autonomi::Wallet`**: Manages user identity and signing capabilities derived from a private key.
3.  **`mutant_lib::storage::Storage`**: The foundational layer interacting directly with the Autonomi network via `autonomi::Client` (obtained from `Network` and `Wallet`). It handles low-level scratchpad operations and maintains internal state like `MapInfo` for tracking allocations.
4.  **`mutant_lib::api::MutAnt`**: The high-level abstraction layer providing the primary key-value API (`store`, `fetch`, `remove`, etc.) exposed by the library. It coordinates operations using the underlying layers and manages user-facing logic and callbacks.

Data is stored and retrieved as raw byte vectors (`Vec<u8>`), allowing the user flexibility in serialization/deserialization.

## Configuration

MutAnt primarily derives its configuration from your locally configured `ant` wallet. It detects available wallets and prompts for selection on the first run, storing the choice in `~/.config/mutant/config.json`. Network selection (Mainnet vs. Devnet/Local) is typically handled via the `--local` flag in the CLI or potentially through `MutAntConfig` in the library.

## License

This project is licensed under the LGPLv3 license. See the [LICENSE](LICENSE) file for details.

## Contributing

*(Add guidelines for contributing to this project here)*
