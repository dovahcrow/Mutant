use base64::DecodeError;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

// --- Event System Definitions ---

/// Callback type used during `get` operations to report progress and allow cancellation.
///
/// The callback receives `GetEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type GetCallback = Arc<
    dyn Fn(
            GetEvent,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<bool, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + Sync,
            >,
        > + Send
        + Sync,
>;

/// Callback type used during initialization (`init`) operations to report progress
/// and handle interactive prompts.
///
/// The callback receives `InitProgressEvent` variants and returns a `Future` that resolves to:
/// - `Ok(Some(true))`: User confirmed action (e.g., create remote index).
/// - `Ok(Some(false))`: User denied action.
/// - `Ok(None)`: Event acknowledged, no specific user action required.
/// - `Err(e)`: Propagate an error from the callback.
pub type InitCallback = Box<
    dyn Fn(
            InitProgressEvent,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Option<bool>, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + Sync,
            >,
        > + Send
        + Sync,
>;

/// Callback type used during `purge` operations to report progress and allow cancellation.
///
/// The callback receives `PurgeEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type PurgeCallback = Arc<
    dyn Fn(
            PurgeEvent,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<bool, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + Sync,
            >,
        > + Send
        + Sync,
>;

/// Callback type used during `sync` operations to report progress and allow cancellation.
///
/// The callback receives `SyncEvent` variants and returns a `Future` that resolves to:
/// - `Ok(true)`: Continue the operation.
/// - `Ok(false)`: Cancel the operation (results in `Error::OperationCancelled`).
/// - `Err(e)`: Propagate an error from the callback.
pub type SyncCallback = Arc<
    dyn Fn(
            SyncEvent,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<bool, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + Sync,
            >,
        > + Send
        + Sync,
>;

/// Events emitted during a `get` operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GetEvent {
    /// Indicates the start of chunk fetching.
    Starting {
        /// Total number of data chunks to be fetched (including index).
        total_chunks: usize,
    },

    /// Indicates that a specific data chunk has been fetched from storage.
    PadsFetched,

    /// Indicates that the `get` operation has completed successfully.
    Complete,
}

/// Events emitted during an `init` (initialization) operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InitProgressEvent {
    /// Indicates the start of the initialization process.
    Starting {
        /// An estimated total number of steps for the initialization.
        total_steps: u64,
    },

    /// Reports progress on a specific step during initialization.
    Step {
        /// The current step number.
        step: u64,
        /// A message describing the current step.
        message: String,
    },

    /// Indicates that user confirmation is required to create a remote index.
    /// The `InitCallback` should return `Ok(Some(true))` to proceed or `Ok(Some(false))` to skip.
    PromptCreateRemoteIndex,

    /// Indicates that the initialization process has failed.
    Failed {
        /// A message describing the failure.
        error_msg: String,
    },

    /// Indicates that the initialization process has completed successfully.
    Complete {
        /// A final message summarizing the outcome.
        message: String,
    },
}

/// Events emitted during a `purge` operation (storage cleanup/verification).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PurgeEvent {
    /// Indicates the start of the `purge` operation.
    Starting {
        /// Total number of pads to be processed.
        total_count: usize,
    },

    /// Indicates that a single pad has been processed (verified or marked for cleanup).
    PadProcessed,

    /// Indicates that the `purge` operation has completed.
    Complete {
        /// Number of pads successfully verified.
        verified_count: usize,
        /// Number of pads that failed verification or encountered errors.
        failed_count: usize,
    },
}

/// Events emitted during a `sync` operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncEvent {
    /// Indicates the remote index is being fetched.
    FetchingRemoteIndex,

    /// Indicates that the remote index is being merged with the local index.
    Merging,

    /// Indicates that the remote index is being pushed to the network.
    PushingRemoteIndex,

    /// Indicates that the remote index is being Verified.
    VerifyingRemoteIndex,

    /// Indicates that the `sync` operation has completed successfully.
    Complete,
}

// --- Task Management System Definitions ---

pub type TaskId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskType {
    Put,
    Get,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PutEvent {
    Starting {
        total_chunks: usize,
        initial_written_count: usize,
        initial_confirmed_count: usize,
        chunks_to_reserve: usize,
    },
    PadReserved,
    PadsWritten,
    PadsConfirmed,
    Complete,
}

pub type PutCallback = Arc<
    dyn Fn(
            PutEvent,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<bool, Box<dyn std::error::Error + Send + Sync>>>
                    + Send
                    + Sync,
            >,
        > + Send
        + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskProgress {
    Put(PutEvent),
    Get(GetEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResult {
    pub error: Option<String>, // Error message if TaskStatus is Failed
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub progress: Option<TaskProgress>,
    pub result: Option<TaskResult>,
}

// --- Protocol Definitions (Requests & Responses) ---

// --- Incoming Requests ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PutRequest {
    pub user_key: String,
    pub source_path: String, // Path to the file on the daemon's filesystem
    pub no_verify: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetRequest {
    pub user_key: String,
    pub destination_path: String, // Path where the fetched file should be saved on the daemon
}

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize, Clone)]
pub struct QueryTaskRequest {
    pub task_id: Uuid,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize, Clone)]
pub struct ListTasksRequest;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RmRequest {
    pub user_key: String,
}

/// Represents all possible requests the client can send to the daemon.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum Request {
    Put(PutRequest),
    Get(GetRequest),
    QueryTask(QueryTaskRequest),
    ListTasks(ListTasksRequest),
    Rm(RmRequest), // Added Rm variant
}

// --- Outgoing Responses ---

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize, Clone)]
pub struct TaskCreatedResponse {
    pub task_id: Uuid,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskUpdateResponse {
    pub task_id: TaskId,
    pub status: TaskStatus,
    pub progress: Option<TaskProgress>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize, Clone)]
pub struct TaskResultResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub result: Option<TaskResult>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize, Clone)]
pub struct TaskListEntry {
    pub task_id: Uuid,
    pub task_type: TaskType,
    pub status: TaskStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorResponse {
    pub error: String,
    pub original_request: Option<String>, // Optional original request string for context
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RmSuccessResponse {
    pub user_key: String,
}

/// Represents all possible responses the daemon can send to the client.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum Response {
    Error(ErrorResponse),
    TaskCreated(TaskCreatedResponse),
    TaskUpdate(TaskUpdateResponse),
    TaskResult(TaskResultResponse),
    TaskList(TaskListResponse),
    RmSuccess(RmSuccessResponse), // Added RmSuccess variant
}

// Helper moved to where Response is used (client/server)
// // Helper to create an ErrorResponse
// impl Response {
//     pub fn error(msg: String, original_request: Option<String>) -> Self {
//         Response::Error(ErrorResponse {
//             error: msg,
//             original_request,
//         })
//     }
// }

// --- Protocol Error Definition ---

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    // Separate deserialization for potentially better client-side error handling
    #[error("JSON deserialization error: {0}")]
    Deserialization(serde_json::Error),

    #[error("Base64 decoding error: {0}")]
    Base64Decode(#[from] DecodeError),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Invalid request format: {0}")]
    InvalidRequest(String),

    #[error("Internal server error: {0}")] // Generic for server-side issues
    InternalError(String),

    #[error("WebSocket error: {0}")] // Can be used by both client/server
    WebSocket(String),
}
