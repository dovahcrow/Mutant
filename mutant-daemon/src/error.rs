use dialoguer;
use std::{io, path::PathBuf};
use thiserror::Error;
use uuid::Uuid;
use warp::reject::Reject;

// Import protocol error for potential mapping/wrapping if needed later
// use mutant_protocol::ProtocolError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("MutAnt library error: {0}")]
    MutAnt(#[from] mutant_lib::error::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] warp::Error),

    #[error("JSON serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Task not found in daemon state: {0}")]
    TaskNotFound(Uuid),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid request structure received: {0}")]
    InvalidRequest(String),

    #[error("Internal daemon error: {0}")]
    Internal(String),

    #[error("Failed to read wallet file at {1}: {0}")]
    WalletRead(io::Error, PathBuf),

    #[error("Could not find Autonomi wallet directory")]
    WalletDirNotFound,

    #[error("Failed to read wallet directory at {1}: {0}")]
    WalletDirRead(io::Error, PathBuf),

    #[error("No wallet files found in {0}")]
    NoWalletsFound(PathBuf),

    #[error("Failed to get user wallet selection: {0}")]
    UserSelectionFailed(dialoguer::Error),

    #[error("No wallet configured or selected")]
    WalletNotSet,

    #[error("Failed to initialize MutAnt: {0}")]
    MutAntInit(String),

    #[error("Failed to read config file at {1}: {0}")]
    ConfigRead(io::Error, PathBuf),

    #[error("Failed to parse config file at {1}: {0}")]
    ConfigParse(serde_json::Error, PathBuf),

    #[error("Failed to write config file at {1}: {0}")]
    ConfigWrite(io::Error, PathBuf),

    #[error("Failed to get base directories: {0}")]
    BaseDirectories(#[from] xdg::BaseDirectoriesError),

    #[error("Failed to decode base64 data: {0}")]
    Base64Decode(base64::DecodeError),
}

// Implement warp::reject::Reject for DaemonError so we can use it in warp filters
impl Reject for Error {}

// Removed From implementation for protocol::Response
/*
impl From<&DaemonError> for mutant_protocol::Response {
    fn from(err: &DaemonError) -> Self {
        // TODO: Map DaemonError variants to appropriate ProtocolError messages
        // or generic error messages before creating the ErrorResponse.
        // Avoid leaking internal details.
        mutant_protocol::Response::Error(mutant_protocol::ErrorResponse {
            error: format!("Server error occurred: {}", err), // Example generic message
            original_request: None,
        })
    }
}
*/

// Keep this helper, it might still be useful internally
pub fn box_to_daemon_error(err: Box<dyn std::error::Error>) -> Error {
    err.into()
}

impl From<Box<dyn std::error::Error>> for Error {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Error::Internal(err.to_string())
    }
}
