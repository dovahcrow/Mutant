// Layer 0: Network Primitives
pub mod adapter;
pub mod client;
pub mod error;
pub mod wallet;

pub use adapter::NetworkAdapter;
pub use error::NetworkError; // Re-export the trait

// Define NetworkChoice here as it's fundamental config
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NetworkChoice {
    Devnet,
    Mainnet,
}

#[cfg(test)]
pub mod integration_tests; // Include integration tests when compiling for tests
