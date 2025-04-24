use base64::DecodeError;
use serde::{Deserialize, Serialize};
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
pub struct TaskProgress {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResult {
    pub data: Option<String>,  // Base64 encoded for Get
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

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize)]
pub struct PutRequest {
    pub user_key: String,
    pub data_b64: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize)]
pub struct GetRequest {
    pub user_key: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize)]
pub struct QueryTaskRequest {
    pub task_id: Uuid,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize)]
pub struct ListTasksRequest;

#[derive(Deserialize, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum Request {
    Put(PutRequest),
    Get(GetRequest),
    QueryTask(QueryTaskRequest),
    ListTasks(ListTasksRequest),
}

// --- Outgoing Responses ---

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskCreatedResponse {
    pub task_id: Uuid,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskUpdateResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub progress: Option<TaskProgress>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskResultResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub result: Option<TaskResult>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskListEntry {
    pub task_id: Uuid,
    pub task_type: TaskType,
    pub status: TaskStatus,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListEntry>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub original_request: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    TaskCreated(TaskCreatedResponse),
    TaskUpdate(TaskUpdateResponse),
    TaskResult(TaskResultResponse),
    TaskList(TaskListResponse),
    Error(ErrorResponse),
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
