// This file will contain the client library implementation

use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc}; // Use std Mutex for simplicity, can switch to tokio::sync::Mutex if needed later

use serde_json;
use url::Url;

use mutant_protocol::{
    ErrorResponse, ListTasksRequest, QueryTaskRequest, Request, Response, Task,
    TaskCreatedResponse, TaskId, TaskListEntry, TaskListResponse, TaskResultResponse, TaskStatus,
    TaskType, TaskUpdateResponse,
};
// use tokio::sync::RwLock; // Removed this - using Rc<RefCell<>> for WASM

use tracing::{debug, error, info, warn};

// Base64 engine
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use futures::channel::oneshot;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

pub mod error;

use crate::error::ClientError;

// Shared state for tasks managed by the client (using Arc<Mutex> for thread safety)
type ClientTaskMap = Arc<Mutex<HashMap<TaskId, Task>>>;

// Remove Ws alias for web_sys::WebSocket
// type Ws = Rc<WebSocket>;

// Type alias for the pending request senders (wrapping Option in Mutex for shared access)
type PendingTaskCreationSender = Arc<Mutex<Option<oneshot::Sender<Result<TaskId, ClientError>>>>>;
type PendingTaskListSender =
    Arc<Mutex<Option<oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>>>>;

#[derive(Debug, Clone, PartialEq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

/// A client for interacting with the Mutant Daemon over WebSocket (Cross-platform implementation).
pub struct MutantClient {
    sender: Option<ewebsock::WsSender>,
    receiver: Option<ewebsock::WsReceiver>,
    tasks: ClientTaskMap,
    pending_task_creation: PendingTaskCreationSender,
    pending_task_list: PendingTaskListSender,
    state: Arc<Mutex<ConnectionState>>,
}

// enum ConnectionState { Connecting, Open, Closed(Option<String>), Error(String) }

impl MutantClient {
    /// Creates a new client instance but does not connect yet.
    pub fn new() -> Self {
        Self {
            sender: None,
            receiver: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            pending_task_creation: Arc::new(Mutex::new(None)),
            pending_task_list: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
        }
    }

    /// Establishes a WebSocket connection to the Mutant Daemon.
    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        if self.sender.is_some() {
            warn!("Already connected or connecting.");
            return Ok(());
        }

        let url = Url::parse(addr).map_err(|e| ClientError::UrlParseError(e))?;
        info!("Connecting to Mutant Daemon at {}", url);

        *self.state.lock().unwrap() = ConnectionState::Connecting;

        let options = ewebsock::Options::default();
        let (sender, receiver) = ewebsock::connect(url.as_str(), options)
            .map_err(|e| ClientError::WebSocketError(e.to_string()))?;

        self.sender = Some(sender);
        self.receiver = Some(receiver);

        *self.state.lock().unwrap() = ConnectionState::Connected;

        Ok(())
    }

    /// Processes a deserialized response from the server
    fn process_response(
        response: Response,
        tasks: &ClientTaskMap,
        pending_task_creation: &PendingTaskCreationSender,
        pending_task_list: &PendingTaskListSender,
    ) {
        debug!("Processing server response: {:?}", response);

        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                debug!("Processing TaskCreated response for task {}", task_id);
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    debug!("Found pending task creation sender for TaskId: {}", task_id);
                    if sender.send(Ok(task_id)).is_err() {
                        warn!("Failed to send TaskCreated response to waiting future (receiver dropped?)");
                    }
                } else {
                    warn!(
                        "Received TaskCreated ({}) but no request was pending.",
                        task_id
                    );
                }
            }
            Response::TaskUpdate(TaskUpdateResponse {
                task_id,
                status,
                progress,
            }) => {
                debug!(
                    "Processing TaskUpdate response for task {} with status {:?}",
                    task_id, status
                );
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    debug!(
                        "Found existing task {}, updating status and progress",
                        task_id
                    );
                    task.status = status;
                    task.progress = progress.clone();
                    info!("Task {} updated: {:?}", task_id, task.status);
                    if let Some(ref progress) = progress {
                        debug!("Task {} progress updated: {:?}", task_id, progress);
                    }
                } else {
                    debug!("Creating new task entry for unknown task {}", task_id);
                    warn!("Received update for unknown task: {}", task_id);
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get,
                            status,
                            progress,
                            result: None,
                        },
                    );
                }
            }
            Response::TaskResult(TaskResultResponse {
                task_id,
                status,
                result,
            }) => {
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.result = result.clone();
                    info!("Task {} finished: {:?}", task_id, task.status);
                } else {
                    warn!("Received result for unknown/untracked task: {}", task_id);
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get, // Unknown
                            status,
                            result,
                            progress: None,
                        },
                    );
                }
            }
            Response::TaskList(TaskListResponse { tasks: task_list }) => {
                // Check if we were waiting for this
                if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    debug!(
                        "Received expected TaskList response ({} tasks)",
                        task_list.len()
                    );
                    if sender.send(Ok(task_list)).is_err() {
                        warn!("Failed to send TaskList response to waiting future (receiver dropped?)");
                    }
                } else {
                    warn!("Received TaskList but no request was pending.");
                }
            }
            Response::Error(ErrorResponse {
                error,
                original_request,
            }) => {
                // Check if this error corresponds to a pending request
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    warn!(
                        "Received Error response while waiting for TaskCreated: {}",
                        error
                    );
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    warn!(
                        "Received Error response while waiting for TaskList: {}",
                        error
                    );
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else {
                    // Generic error, not tied to a specific pending request we are handling here
                    error!(
                        "Received server error: {}. Original request: {:?}",
                        error, original_request
                    );
                }
            }
        }
    }

    /// Sends a request over the WebSocket.
    async fn send_request(&mut self, request: Request) -> Result<(), ClientError> {
        let sender = self.sender.as_mut().ok_or(ClientError::NotConnected)?;

        let json =
            serde_json::to_string(&request).map_err(|e| ClientError::SerializationError(e))?;

        sender.send(ewebsock::WsMessage::Text(json));

        Ok(())
    }

    // --- Public API Methods ---
    // These need rethinking for WASM's async/callback model.
    // A simple request/response map or channels might be needed.

    pub async fn put(&mut self, user_key: &str, data: &[u8]) -> Result<TaskId, ClientError> {
        // Ensure only one put/get request is pending
        if self.pending_task_creation.lock().unwrap().is_some() {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_creation.lock().unwrap() = Some(sender);

        let data_b64 = BASE64_STANDARD.encode(data);
        let req = Request::Put(mutant_protocol::PutRequest {
            user_key: user_key.to_string(),
            data_b64,
        });

        match self.send_request(req).await {
            Ok(_) => {
                debug!("Put request sent, waiting for TaskCreated response...");
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskCreated channel canceled".to_string())
                })?
            }
            Err(e) => {
                // Request failed, clear the pending sender
                *self.pending_task_creation.lock().unwrap() = None;
                Err(e)
            }
        }
    }

    pub async fn get(&mut self, user_key: &str) -> Result<TaskId, ClientError> {
        if self.pending_task_creation.lock().unwrap().is_some() {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_creation.lock().unwrap() = Some(sender);

        let req = Request::Get(mutant_protocol::GetRequest {
            user_key: user_key.to_string(),
        });

        match self.send_request(req).await {
            Ok(_) => {
                debug!("Get request sent, waiting for TaskCreated response...");
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskCreated channel canceled".to_string())
                })?
            }
            Err(e) => {
                *self.pending_task_creation.lock().unwrap() = None;
                Err(e)
            }
        }
    }

    pub async fn list_tasks(&mut self) -> Result<Vec<TaskListEntry>, ClientError> {
        if self.pending_task_list.lock().unwrap().is_some() {
            return Err(ClientError::InternalError(
                "Another list_tasks request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_list.lock().unwrap() = Some(sender);

        let req = Request::ListTasks(ListTasksRequest);

        match self.send_request(req).await {
            Ok(_) => {
                debug!("ListTasks request sent, waiting for TaskList response...");
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskList channel canceled".to_string())
                })?
            }
            Err(e) => {
                *self.pending_task_list.lock().unwrap() = None;
                Err(e)
            }
        }
    }

    pub async fn query_task(&mut self, task_id: TaskId) -> Result<(), ClientError> {
        // Sends the request, result comes back to onmessage and updates the internal map.
        let req = Request::QueryTask(QueryTaskRequest { task_id });
        self.send_request(req).await
    }

    pub async fn next_response(&mut self) -> Option<Result<Response, ClientError>> {
        if let Some(receiver) = &mut self.receiver {
            match receiver.try_recv() {
                Some(event) => {
                    debug!("Received WebSocket event: {:?}", event);
                    match event {
                        ewebsock::WsEvent::Message(msg) => {
                            if let ewebsock::WsMessage::Text(text) = msg {
                                debug!("Received WebSocket text message: {}", text);
                                match serde_json::from_str::<Response>(&text) {
                                    Ok(response) => {
                                        debug!(
                                            "Successfully deserialized response: {:?}",
                                            response
                                        );
                                        Self::process_response(
                                            response.clone(),
                                            &self.tasks,
                                            &self.pending_task_creation,
                                            &self.pending_task_list,
                                        );
                                        Some(Ok(response))
                                    }
                                    Err(e) => {
                                        error!("Failed to deserialize response: {}", e);
                                        Some(Err(ClientError::DeserializationError(e)))
                                    }
                                }
                            } else {
                                debug!("Received non-text WebSocket message");
                                None
                            }
                        }
                        ewebsock::WsEvent::Error(e) => {
                            error!("WebSocket error: {}", e);
                            Some(Err(ClientError::WebSocketError(e.to_string())))
                        }
                        ewebsock::WsEvent::Closed => {
                            debug!("WebSocket connection closed");
                            None
                        }
                        ewebsock::WsEvent::Opened => {
                            debug!("WebSocket connection opened");
                            None
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        }
    }

    // --- Accessor methods for internal state ---

    pub fn get_task_status(&self, task_id: TaskId) -> Option<TaskStatus> {
        self.tasks
            .lock()
            .unwrap()
            .get(&task_id)
            .map(|t| t.status.clone())
    }

    pub fn get_task_result(&self, task_id: TaskId) -> Option<mutant_protocol::TaskResult> {
        self.tasks
            .lock()
            .unwrap()
            .get(&task_id)
            .and_then(|t| t.result.clone())
    }
}

impl Clone for MutantClient {
    fn clone(&self) -> Self {
        Self {
            sender: None,
            receiver: None,
            tasks: self.tasks.clone(),
            pending_task_creation: self.pending_task_creation.clone(),
            pending_task_list: self.pending_task_list.clone(),
            state: self.state.clone(),
        }
    }
}

// Need to run this once at the start of the WASM application
pub fn set_panic_hook() {
    console_error_panic_hook::set_once();
}
