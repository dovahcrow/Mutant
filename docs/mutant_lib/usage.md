# MutAnt Library: Usage Examples

This document provides practical examples of how to use the `mutant-lib` API.

**Note:** These examples assume you have the necessary `autonomi` setup (wallet, private key) and are running within a `tokio` runtime.

```rust
use mutant_lib::{
    MutAnt,
    Error,
    tokio, 
};
use std::sync::Arc;
use std::time::Duration;


async fn run_mutant_examples() -> Result<(), Box<dyn std::error::Error>> {
    let private_key_hex = "your_autonomi_account_private_key_hex_string_here";

    let (mutant, _init_handle) = MutAnt::init(
        private_key_hex.to_string(),
    )
    .await?;

    mutant.store("Hello", "MutAnt world!".as_bytes()).await?;

    let fetched_data1 = mutant.fetch("Hello").await?;
    assert_eq!(b"MutAnt world!", fetched_data1);

    let keys = mutant.list_keys().await?;
    assert!(keys.contains(&"Hello"));

    mutant.update("Hello", "MutAnt world! updated".as_bytes()).await?;

    let fetched_updated_data1 = mutant.fetch("Hello").await?;
    assert_eq!(b"MutAnt world! updated", fetched_updated_data1);

    mutant.remove("Hello").await?;

    match mutant.fetch("Hello").await {
        Err(Error::KeyNotFound(_)) => { /* Expected */ }
        Ok(_) => panic!("Key should have been removed!"),
        Err(e) => return Err(e.into()),
    }

    let keys_after_remove = mutant.list_keys().await?;
    assert!(!keys_after_remove.contains(&"Hello"));


    Ok(())
}

#[tokio::main]
async fn main() {
    env_logger::init(); 

    if let Err(e) = run_mutant_examples().await {
        eprintln!("\nError during example run: {}", e);
            let mut source = e.source();
            while let Some(cause) = source {
                eprintln!("  Caused by: {}", cause);
                source = cause.source();
            }
            std::process::exit(1);
        }
    }
}

```

**Before Running:**

1.  Replace `"your_autonomi_account_private_key_hex_string_here"` with the corresponding private key for that wallet.
2.  Ensure you have configured the `autonomi::Client` appropriately (e.g., network endpoints) if not using defaults.
3.  Add `mutant-lib`, `tokio`, `env_logger`, and `futures` (if not already present via `mutant-lib` re-exports) to your `Cargo.toml` dependencies. 