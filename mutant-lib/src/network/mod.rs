pub mod adapter;
pub mod client;
pub mod error;
pub mod wallet;

pub use adapter::NetworkAdapter;
pub use error::NetworkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NetworkChoice {
    Devnet,
    Mainnet,
}

#[cfg(test)]
pub mod integration_tests;
