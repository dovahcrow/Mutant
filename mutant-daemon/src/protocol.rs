use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Incoming Requests ---

#[derive(Deserialize, Debug)]
pub struct PutRequest {
    pub user_key: String,
    pub data_b64: String, // Base64 encoded data
                          // Assuming private, Medium storage for now
}

#[derive(Deserialize, Debug)]
pub struct GetRequest {
    pub user_key: String,
}

#[derive(Deserialize, Debug)]
pub struct QueryTaskRequest {
    pub task_id: Uuid,
}

#[derive(Deserialize, Debug)]
pub struct ListTasksRequest; // No fields needed

#[derive(Deserialize, Debug)]
#[serde(tag = "type")] // Use a 'type' field to distinguish requests
pub enum Request {
    Put(PutRequest),
    Get(GetRequest),
    QueryTask(QueryTaskRequest),
    ListTasks(ListTasksRequest),
}

// --- Outgoing Responses ---

#[derive(Serialize, Debug)]
pub struct TaskCreatedResponse {
    pub task_id: Uuid,
}

#[derive(Serialize, Debug)]
pub struct TaskUpdateResponse {
    pub task_id: Uuid,
    pub status: crate::TaskStatus,
    pub progress: Option<crate::TaskProgress>,
}

#[derive(Serialize, Debug)]
pub struct TaskResultResponse {
    pub task_id: Uuid,
    pub status: crate::TaskStatus,
    pub result: Option<crate::TaskResult>,
}

#[derive(Serialize, Debug)]
pub struct TaskListEntry {
    pub task_id: Uuid,
    pub task_type: crate::TaskType,
    pub status: crate::TaskStatus,
}

#[derive(Serialize, Debug)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListEntry>,
}

#[derive(Serialize, Debug)]
pub struct ErrorResponse {
    pub error: String,
    pub original_request: Option<String>, // Optionally include the request that failed
}

#[derive(Serialize, Debug)]
#[serde(tag = "type")] // Use a 'type' field to distinguish responses
pub enum Response {
    TaskCreated(TaskCreatedResponse),
    TaskUpdate(TaskUpdateResponse),
    TaskResult(TaskResultResponse),
    TaskList(TaskListResponse),
    Error(ErrorResponse),
}

// Helper to create an ErrorResponse
impl Response {
    pub fn error(msg: String, original_request: Option<String>) -> Self {
        Response::Error(ErrorResponse {
            error: msg,
            original_request,
        })
    }
}
