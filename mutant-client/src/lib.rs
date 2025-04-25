// This file will contain the client library implementation

use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc}; // Use std Mutex for simplicity, can switch to tokio::sync::Mutex if needed later

use mutant_protocol::{
    ErrorResponse, GetRequest, ListTasksRequest, PutRequest, QueryTaskRequest, Request, Response,
    Task, TaskCreatedResponse, TaskId, TaskListEntry, TaskListResponse, TaskResultResponse,
    TaskStatus, TaskType, TaskUpdateResponse,
};
// use tokio::sync::RwLock; // Removed this - using Rc<RefCell<>> for WASM

use tracing::{debug, error, info, warn};
use url::Url;

// Base64 engine
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use futures::channel::oneshot;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

#[cfg(not(target_arch = "wasm32"))]
use tokio::spawn;

mod message;

use crate::error::Error;
use crate::message::{Message, MessageType};

pub mod error;
pub use error::ClientError;

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
    Error(String),
}

/// A client for interacting with the Mutant Daemon over WebSocket (Cross-platform implementation).
pub struct MutantClient {
    // Replace web_sys::WebSocket with ewebsock sender/receiver
    // ws: Option<Ws>,
    sender: Option<ewebsock::WsSender>,
    // Receiver needs careful handling - maybe store it temporarily during connect?
    // Or it needs to be moved to the receiving task.
    // Let's omit receiver from the struct for now, it will be handled by the task.
    tasks: ClientTaskMap,
    pending_task_creation: PendingTaskCreationSender,
    pending_task_list: PendingTaskListSender,
    // Remove WASM callbacks
    // _onopen_callback: Option<Closure<dyn FnMut()>>,
    // _onmessage_callback: Option<Closure<dyn FnMut(MessageEvent)>>,
    // _onerror_callback: Option<Closure<dyn FnMut(ErrorEvent)>>,
    // _onclose_callback: Option<Closure<dyn FnMut(CloseEvent)>>,

    // TODO: Maybe add a state field? e.g., Arc<Mutex<ConnectionState>>
    state: Arc<Mutex<ConnectionState>>,
}

// enum ConnectionState { Connecting, Open, Closed(Option<String>), Error(String) }

impl MutantClient {
    /// Creates a new client instance but does not connect yet.
    pub fn new() -> Self {
        Self {
            // ws: None, // Removed
            sender: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            pending_task_creation: Arc::new(Mutex::new(None)),
            pending_task_list: Arc::new(Mutex::new(None)),
            // Callbacks removed
            // _onopen_callback: None,
            // _onmessage_callback: None,
            // _onerror_callback: None,
            // _onclose_callback: None,
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
        }
    }

    /// Establishes a WebSocket connection to the Mutant Daemon.
    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        if self.sender.is_some() {
            warn!("Already connected or connecting.");
            return Ok(());
        }

        let url = Url::parse(addr)?;
        info!("Connecting to Mutant Daemon at {}", url);

        *self.state.lock().unwrap() = ConnectionState::Connecting;

        let options = ewebsock::Options::default();
        let (sender, receiver) = ewebsock::connect(url.as_str(), options)
            .map_err(|e| ClientError::WebSocketError(e.to_string()))?;

        // Store sender for future use
        self.sender = Some(sender);

        // Clone shared state for the receiver task
        let tasks = self.tasks.clone();
        let pending_task_creation = self.pending_task_creation.clone();
        let pending_task_list = self.pending_task_list.clone();
        let state = self.state.clone();

        // Spawn a task to handle incoming messages
        #[cfg(target_arch = "wasm32")]
        spawn_local(async move {
            Self::handle_receiver_loop(
                receiver,
                tasks,
                pending_task_creation,
                pending_task_list,
                state,
            )
            .await;
        });

        #[cfg(not(target_arch = "wasm32"))]
        spawn(async move {
            Self::handle_receiver_loop(
                receiver,
                tasks,
                pending_task_creation,
                pending_task_list,
                state,
            )
            .await;
        });

        Ok(())
    }

    /// Internal function to handle the WebSocket receiver loop
    async fn handle_receiver_loop(
        mut receiver: ewebsock::WsReceiver,
        tasks: ClientTaskMap,
        pending_task_creation: PendingTaskCreationSender,
        pending_task_list: PendingTaskListSender,
        state: Arc<Mutex<ConnectionState>>,
    ) {
        while let Some(event) = receiver.try_recv() {
            match event {
                ewebsock::WsEvent::Opened => {
                    info!("WebSocket connection opened");
                    *state.lock().unwrap() = ConnectionState::Connected;
                }
                ewebsock::WsEvent::Message(msg) => {
                    if let ewebsock::WsMessage::Text(text) = msg {
                        match serde_json::from_str::<Response>(&text) {
                            Ok(response) => {
                                Self::process_response(
                                    response,
                                    &tasks,
                                    &pending_task_creation,
                                    &pending_task_list,
                                );
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to deserialize server message: {}. Text: {}",
                                    e, text
                                );
                            }
                        }
                    }
                }
                ewebsock::WsEvent::Error(e) => {
                    let err_str = e.to_string();
                    error!("WebSocket error: {}", err_str);
                    *state.lock().unwrap() = ConnectionState::Error(err_str.clone());
                    // Signal error to pending requests
                    if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                        let _ = sender.send(Err(ClientError::WebSocketError(err_str.clone())));
                    }
                    if let Some(sender) = pending_task_list.lock().unwrap().take() {
                        let _ = sender.send(Err(ClientError::WebSocketError(err_str)));
                    }
                }
                ewebsock::WsEvent::Closed => {
                    info!("WebSocket connection closed");
                    *state.lock().unwrap() = ConnectionState::Disconnected;
                    // Signal closure to pending requests
                    if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                        let _ = sender.send(Err(ClientError::NotConnected));
                    }
                    if let Some(sender) = pending_task_list.lock().unwrap().take() {
                        let _ = sender.send(Err(ClientError::NotConnected));
                    }
                    break;
                }
            }
        }
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
                // Check if we were waiting for this
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    debug!(
                        "Received expected TaskCreated response for TaskId: {}",
                        task_id
                    );
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
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.progress = progress.clone();
                    info!("Task {} updated: {:?}", task_id, task.status);
                } else {
                    warn!("Received update for unknown task: {}", task_id);
                    // Optionally add a placeholder task if we want to track all updates
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get, // Unknown
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
        if let Some(sender) = &mut self.sender {
            let json = serde_json::to_string(&request)?;
            sender.send(ewebsock::WsMessage::Text(json));
            Ok(())
        } else {
            Err(ClientError::NotConnected)
        }
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

// Need to run this once at the start of the WASM application
pub fn set_panic_hook() {
    console_error_panic_hook::set_once();
}
