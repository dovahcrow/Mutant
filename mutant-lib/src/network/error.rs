use ant_networking::GetRecordError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Failed to initialize Autonomi network: {0}")]
    NetworkInitError(String),

    #[error("Failed to create Autonomi wallet: {0}")]
    WalletError(String),

    #[error("Failed to initialize Autonomi client: {0}")]
    ClientInitError(String),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),

    #[error("Internal network adapter error: {0}")]
    InternalError(String),

    #[error("Invalid private key input: {0}")]
    InvalidKeyInput(String),

    #[error("State inconsistency detected: {0}")]
    InconsistentState(String),

    #[error("Scratchpad not found: {0}")]
    GetError(GetRecordError),

    #[error("Network operation timed out: {0}")]
    Timeout(String),
}
