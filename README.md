# Mutant: Decentralized P2P Mutable Key-Value Storage for [Autonomi](https://autonomi.com)

<!-- Badges -->
[![Build Status](https://github.com/Champii/Mutant/actions/workflows/ci.yml/badge.svg)](https://github.com/Champii/Mutant/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/mutant-lib.svg)](https://crates.io/crates/mutant-lib)
[![Docs.rs](https://docs.rs/mutant-lib/badge.svg)](https://docs.rs/mutant-lib)
[![License: LGPLv3](https://img.shields.io/badge/license-LGPLv3-blue.svg)](LICENSE)

> **Mutant** is a public/private mutable key-value store on the Autonomi decentralized storage network, featuring resumable uploads, local index caching, and a powerful async Rust API + CLI.

<p align="center">
  <img src="docs/screenshots/cover.png" alt="cover" width="40%">
</p>
<p align="center">
  <img src="docs/screenshots/demo.gif" alt="demo" width="100%">
</p>



**Note:** No LLMs were harmed in the making of this project.

## Table of Contents
1. [Core Concepts](#core-concepts)
2. [Getting Started](#getting-started)
    1. [Prerequisites](#prerequisites)
    2. [Setup Rust Toolchain](#setup-rust-toolchain)
    3. [Setup `ant` Wallet](#setup-ant-wallet)
3. [Installation](#installation)
4. [Quick start demo](#quick-start-demo)
5. [Command-Line Interface (CLI)](#command-line-interface-cli)
    1. [CLI Usage Examples](#cli-usage-examples)
    2. [Basic Usage](#basic-usage)
        1. [Store/fetch private data](#storefetch-private-data)
        2. [Store/fetch public data](#storefetch-public-data)
        3. [Pipes and redirects](#pipes-and-redirects)
        4. [Stats and debug](#stats-and-debug)
    3. [Screenshots](#screenshots)
6. [Library Usage](#library-usage)
    1. [Fetching Public Data (Keyless Initialization)](#fetching-public-data-keyless-initialization)
7. [Development and Testing](#development-and-testing)
    1. [Local Testnet Management (`scripts/manage_local_testnet.sh`)](#local-testnet-management-scriptsmanage_local_testnetsh)
    2. [Running Integration Tests (`scripts/run_tests_with_env.sh`)](#running-integration-tests-scriptsrun_tests_with_envsh)
8. [Migration](#migration)
9. [Architecture Overview](#architecture-overview)
10. [Configuration](#configuration)
11. [License](#license)
12. [Contributing](#contributing)

## Core Concepts

*   **Decentralized:** Data is stored on the Autonomi decentralized storage network, not a centralized server. This means that no one can censor you, and you are in control of your own data, and it is accessible for anyone from anywhere. (If you wish so)
*   **Mutable:** Data can be updated and deleted, and the changes are persisted on the network.
*   **Key-Value Storage:** Offers a clean, asynchronous key-value interface (`get`, `put`, `rm`).
*   **Public/Private Uploads:** Store data publicly to share with others (no encryption) or store privately (encrypted with your private key).
*   **Resumable Uploads:** Automatic resume of interrupted uploads; pick up right where you left off.
*   **Configurable Upload Size:** Split the data into smaller of bigger chunks to optimize the upload speed and storage costs (default is 2MiB).
*   **Fetch History:** Keep track of the public data you've fetched to re-fetch it later.
*   **Efficient Space Reuse:** Frees and reuses storage pads, minimizing storage costs.
*   **Local Cache Index:** Fast local lookups and seamless remote synchronization.
*   **Sync:** Synchronize your local index with the remote storage.
*   **Recycling Bad Pads:** Automatically recycle bad pads to stay performant.
*   **Health Check:** Perform a health check on keys and reupload pads that give an error.
*   **Async-first Design:** Built on `tokio` for high-performance non-blocking operations.
*   **Dual Interface:** Use as a Rust library (`mutant-lib`) or via the `mutant` CLI.

## Getting Started

### Prerequisites

*   Rust Toolchain (latest stable recommended)
*   `ant` CLI configured with a wallet (see below)

### Setup Rust Toolchain

This will install rustup

```bash
$> curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

This will install the latest stable version of the Rust toolchain and cargo

```bash
$> rustup install nightly
```

### Setup `ant` Wallet

#### If you just want to fetch public data, you can skip this section.

Before using `mutant` to actually store data, you need to have an `ant` wallet configured for the target network (Mainnet by default, or Devnet if using the `--local` flag). If you don't have `ant` installed, you can get it using [antup](https://github.com/maidsafe/antup):

```bash
$> curl -sSf https://raw.githubusercontent.com/maidsafe/antup/main/install.sh | sh
```

This will install the `ant` CLI and configure it for the Mainnet.

```bash
$> antup client
```

Once `ant` is installed, if you haven't already, you can import your existing Ethereum/ANT wallet's private key using the `ant` CLI:

```bash
$> ant wallet import YOUR_PRIVATE_KEY_HERE
```

Replace `YOUR_PRIVATE_KEY_HERE` with your actual private key. `mutant` will automatically detect and use this wallet.

Alternatively, you can create a new empty wallet using `ant wallet create` and fund it with the necessary ANT or ETH later.

MutAnt will look for your ant wallets and ask you which one you want to use if you have multiple on the first run, then save your choice in `~/.config/mutant/config.json`.

## Installation

```bash
$> cargo install mutant
```

## Quick start demo

You can fetch the daily meme of the day with the following command:

```bash
$> mutant get -p 80a9296e666d9f6d0dad8672e8a17fe15d00bae67e35ae4a471aabc2351ac85c4af654be0ea4c2ababf9a788b751ceac > daily_meme.jpg
```

## Command-Line Interface (CLI)

MutAnt includes the `mutant` command for convenient command-line access.

**CLI Usage Examples:**

```bash
$> mutant --help
```

```text
Distributed mutable key value storage over the Autonomi network

Usage: mutant [OPTIONS] <COMMAND>

Commands:
  put           Store a value associated with a key
  get           Retrieve a value associated with a key
  rm            Remove a key-value pair
  ls            List stored keys
  export        Export all scratchpad private key to a file
  import        Import scratchpad private key from a file
  stats         Show storage statistics
  sync          Synchronize local index cache with remote storage
  purge         Perform a get check on scratchpads that should have been created but failed at some point. Removes the pads that are not found.
  health-check  Perform a health check on scratchpads that should have been created but cannot be retrieved. Recycles the pads that are not found.
  help          Print this message or the help of the given subcommand(s)

Options:
  -l, --local    Use local network (Devnet)
  -q, --quiet    Suppress progress bar output
  -h, --help     Print help
  -V, --version  Print version
```

**Basic Usage:**

#### Store/fetch private data

```bash
# Store a value directly
$> mutant put mykey "my value"

# Get a value and print to stdout
$> mutant get mykey
# Output: my value

# Update a value (you can use the shorter -f)
$> mutant put mykey "my new value" --force

# Remove a value
$> mutant rm mykey
```

#### Store/fetch public data

```bash
# Store data publicly (no encryption) under a name
$> mutant put -p my_key "some public content"
# Output: 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef

# Get your own public data by name
$> mutant get my_key
# Output: some public content

# Get public data by address
$> mutant get -p 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
# Output: some public content

# You can update it all the same as the private data
$> mutant put -p my_key "some updated public content" --force
```

#### Pipes and redirects

```bash
# Store a value from stdin (e.g., piping a file)
$> cat data.txt | mutant put mykey2

$> mutant get mykey2 > fetched_data.txt
```

#### Stats and debug


```bash
# List stored keys
$> mutant ls
# test         57 pads 55.80 MiB  (public @ 858105ad390518c26a75e48fdfe87358a76fb24a06b1fb04b634af317ab39efcf8faf95b0d4c5eaac01a3a3fe3f8f6d5)
# mykey        1 pad   1.00 KiB  

# Sync local index with remote storage
$> mutant sync

# View storage statistics
$> mutant stats
```


## Library Usage

Add `mutant-lib` and its dependencies to your `Cargo.toml`:

```toml
[dependencies]
mutant-lib = "0.5.0" # Or the version you need
tokio = { version = "1", features = ["full"] }
```

**Library Example:**

This example demonstrates initializing the necessary components and performing basic private store/fetch operations. It assumes you have an `ant` wallet setup.

```rust
use mutant_lib::{MutAnt, MutAntConfig, Error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Replace with your actual private key (hex format, with or without 0x prefix)
    let private_key_hex = "0xYOUR_PRIVATE_KEY_HEX";

    let mut mutant = MutAnt::init(private_key_hex).await?;

    mutant.put("greeting", b"hello world", StorageMode::Medium, false).await?;

    let fetched_value = mutant.get("greeting").await?;

    println!("Fetched value: {}", String::from_utf8_lossy(&fetched_value));

    mutant.rm("greeting").await?;

    Ok(())
}
```

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
    match public_fetcher.get_public(public_address).await {
        Ok(data) => println!("Fetched public data: {} bytes", data.len()),
        Err(e) => eprintln!("Failed to fetch public data: {}", e),
    }

    Ok(())
}

This keyless instance is optimized for fetching public data and cannot perform operations requiring a private key (like `put`, `rm`, `ls`, etc).