use dialoguer;
use std::{io, path::PathBuf};
use thiserror::Error;
use warp::reject::Reject;

#[derive(Error, Debug)]
pub enum Error {
    #[error("MutAnt library error: {0}")]
    MutAnt(#[from] mutant_lib::error::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] warp::Error),

    #[error("JSON serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("IO error (formatted): {0}")]
    IoError(String),

    #[error("Library error (formatted): {0}")]
    LibError(mutant_lib::error::Error),

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

    #[error("Failed to get base directories: {0}")]
    BaseDirectories(#[from] xdg::BaseDirectoriesError),
}

// Implement warp::reject::Reject for DaemonError so we can use it in warp filters
impl Reject for Error {}

impl From<Box<dyn std::error::Error>> for Error {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Error::Internal(err.to_string())
    }
}
