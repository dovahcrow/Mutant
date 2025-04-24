use thiserror::Error;
use uuid::Uuid;
use warp::reject::Reject;

#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("MutAnt library error: {0}")]
    MutAnt(#[from] mutant_lib::error::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] warp::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Base64 decoding error: {0}")]
    Base64Decode(#[from] base64::DecodeError),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid request format: {0}")]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

// Implement warp::reject::Reject for DaemonError so we can use it in warp filters
impl Reject for DaemonError {}

// Helper function to convert DaemonError to a protocol::Response::Error
impl From<&DaemonError> for crate::protocol::Response {
    fn from(err: &DaemonError) -> Self {
        crate::protocol::Response::error(err.to_string(), None)
    }
}

// Allow easy conversion from Box<dyn std::error::Error>
pub fn box_to_daemon_error(err: Box<dyn std::error::Error>) -> DaemonError {
    // Attempt to downcast to known error types if possible,
    // otherwise wrap as an Internal error.
    // This is basic; more sophisticated downcasting could be added.
    DaemonError::Internal(err.to_string())
}
