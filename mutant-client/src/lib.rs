// This file will contain the client library implementation

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc}; // Use std Mutex for simplicity, can switch to tokio::sync::Mutex if needed later

use serde_json;
use url::Url;

use mutant_protocol::{
    ErrorResponse, ListTasksRequest, QueryTaskRequest, Request, Response, Task,
    TaskCreatedResponse, TaskId, TaskListEntry, TaskListResponse, TaskProgress, TaskResult,
    TaskResultResponse, TaskStatus, TaskType, TaskUpdateResponse,
};
// use tokio::sync::RwLock; // Removed this - using Rc<RefCell<>> for WASM

use log::{debug, error, info, warn};

// Base64 engine
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use futures::channel::oneshot;
use tokio::sync::mpsc;

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

pub type CompletionReceiver = oneshot::Receiver<Result<TaskResult, ClientError>>;
pub type ProgressReceiver = mpsc::UnboundedReceiver<Result<TaskProgress, ClientError>>;

type CompletionSender = oneshot::Sender<Result<TaskResult, ClientError>>;
type ProgressSender = mpsc::UnboundedSender<Result<TaskProgress, ClientError>>;

type TaskChannels = (CompletionSender, ProgressSender);
type TaskChannelsMap = Arc<Mutex<HashMap<TaskId, TaskChannels>>>;

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
    task_channels: TaskChannelsMap,
    pending_task_creation: PendingTaskCreationSender,
    pending_task_list: PendingTaskListSender,
    state: Arc<Mutex<ConnectionState>>,
}

impl MutantClient {
    /// Creates a new client instance but does not connect yet.
    pub fn new() -> Self {
        info!("Creating new MutantClient instance");
        Self {
            sender: None,
            receiver: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            task_channels: Arc::new(Mutex::new(HashMap::new())),
            pending_task_creation: Arc::new(Mutex::new(None)),
            pending_task_list: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
        }
    }

    /// Establishes a WebSocket connection to the Mutant Daemon.
    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        info!("Attempting to connect to {}", addr);
        if self.sender.is_some() {
            warn!("Already connected or connecting.");
            return Ok(());
        }

        let url = Url::parse(addr).map_err(|e| ClientError::UrlParseError(e))?;
        info!("Connecting to Mutant Daemon at {}", url);

        *self.state.lock().unwrap() = ConnectionState::Connecting;

        let options = ewebsock::Options::default();
        info!("Establishing WebSocket connection...");
        let (sender, receiver) = ewebsock::connect(url.as_str(), options)
            .map_err(|e| ClientError::WebSocketError(e.to_string()))?;

        info!("WebSocket connection established successfully");
        self.sender = Some(sender);
        self.receiver = Some(receiver);
        println!("Receiver is {:?}", self.receiver.is_some());

        *self.state.lock().unwrap() = ConnectionState::Connected;

        let mut client_clone = self.partial_take_receiver();
        println!(
            "Client clone receiver is {:?}",
            client_clone.receiver.is_some()
        );
        #[cfg(target_arch = "wasm32")]
        spawn_local(async move {
            while let Some(response) = client_clone.next_response().await {
                if let Err(e) = response {
                    error!("Error processing response: {:?}", e);
                }
            }
        });

        #[cfg(not(target_arch = "wasm32"))]
        tokio::spawn(async move {
            println!("Starting tokio spawn");
            println!(
                "Client clone receiver is {:?}",
                client_clone.receiver.is_some()
            );
            while let Some(response) = client_clone.next_response().await {
                if let Err(e) = response {
                    error!("Error processing response: {:?}", e);
                }
            }
        });

        Ok(())
    }

    /// Processes a deserialized response from the server
    fn process_response(
        response: Response,
        tasks: &ClientTaskMap,
        task_channels: &TaskChannelsMap,
        pending_task_creation: &PendingTaskCreationSender,
        pending_task_list: &PendingTaskListSender,
    ) {
        info!("Processing server response: {:?}", response);

        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                info!("Task created with ID: {}", task_id);
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    info!("Found pending task creation sender for TaskId: {}", task_id);
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
                info!(
                    "Task {} updated - Status: {:?}, Progress: {:?}",
                    task_id, status, progress
                );
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    info!(
                        "Updating existing task {} - New status: {:?}",
                        task_id, status
                    );
                    task.status = status;
                    task.progress = progress.clone();

                    if let Some(progress) = progress {
                        info!("Task {} progress updated: {:?}", task_id, progress);
                        if let Some((_, progress_tx)) = task_channels.lock().unwrap().get(&task_id)
                        {
                            let _ = progress_tx.send(Ok(progress));
                        }
                    }
                } else {
                    info!("Creating new task entry for unknown task {}", task_id);
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
                info!(
                    "Received task result - ID: {}, Status: {:?}, Result: {:?}",
                    task_id, status, result
                );
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    info!("Updating task {} with final result", task_id);
                    task.status = status;
                    task.result = result.clone();

                    if let Some((completion_tx, _)) = task_channels.lock().unwrap().remove(&task_id)
                    {
                        let _ = completion_tx.send(Ok(result.unwrap_or_else(|| TaskResult {
                            data: None,
                            error: None,
                        })));
                    }
                } else {
                    info!("Creating new task entry for completed task {}", task_id);
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get,
                            status,
                            result,
                            progress: None,
                        },
                    );
                }
            }
            Response::TaskList(TaskListResponse { tasks: task_list }) => {
                info!("Received task list with {} tasks", task_list.len());
                if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    info!("Sending task list to waiting receiver");
                    if sender.send(Ok(task_list)).is_err() {
                        warn!("Failed to send TaskList response (receiver dropped)");
                    }
                } else {
                    warn!("Received TaskList but no request was pending");
                }
            }
            Response::Error(ErrorResponse {
                error,
                original_request,
            }) => {
                error!(
                    "Server error received: {}. Original request: {:?}",
                    error, original_request
                );
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    error!("Error occurred during task creation: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    error!("Error occurred during task list request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                }
            }
        }
    }

    /// Sends a request over the WebSocket.
    async fn send_request(&mut self, request: Request) -> Result<(), ClientError> {
        info!("Preparing to send request: {:?}", request);
        let sender = self.sender.as_mut().ok_or_else(|| {
            error!("Attempted to send request while not connected");
            ClientError::NotConnected
        })?;

        let json = serde_json::to_string(&request).map_err(|e| {
            error!("Failed to serialize request: {}", e);
            ClientError::SerializationError(e)
        })?;

        info!("Sending request to server");
        sender.send(ewebsock::WsMessage::Text(json));
        info!("Request sent successfully");

        Ok(())
    }

    // --- Public API Methods ---
    // These need rethinking for WASM's async/callback model.
    // A simple request/response map or channels might be needed.

    pub async fn put<'a>(
        &'a mut self,
        user_key: &str,
        data: &[u8],
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + 'a,
            ProgressReceiver,
        ),
        ClientError,
    > {
        info!("Starting put operation for key: {}", user_key);

        let (completion_tx, completion_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let (task_creation_tx, task_creation_rx) = oneshot::channel();
        *self.pending_task_creation.lock().unwrap() = Some(task_creation_tx);

        let data_b64 = BASE64_STANDARD.encode(data);
        info!("Encoded {} bytes of data to base64", data.len());

        let req = Request::Put(mutant_protocol::PutRequest {
            user_key: user_key.to_string(),
            data_b64,
        });

        let start_task = async move {
            match self.send_request(req).await {
                Ok(_) => {
                    info!("Put request sent, waiting for TaskCreated response...");
                    let task_id = task_creation_rx.await.map_err(|_| {
                        error!("TaskCreated channel canceled");
                        ClientError::InternalError("TaskCreated channel canceled".to_string())
                    })??;

                    info!("Task created with ID: {}, setting up channels", task_id);
                    self.task_channels
                        .lock()
                        .unwrap()
                        .insert(task_id, (completion_tx, progress_tx));

                    completion_rx.await.map_err(|_| {
                        error!("Completion channel canceled");
                        ClientError::InternalError("Completion channel canceled".to_string())
                    })?
                }
                Err(e) => {
                    error!("Failed to send put request: {:?}", e);
                    *self.pending_task_creation.lock().unwrap() = None;
                    Err(e)
                }
            }
        };

        Ok((start_task, progress_rx))
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
        println!("Receiver is {:?}", self.receiver.is_some());
        if let Some(receiver) = &mut self.receiver {
            println!("Receiver is Some");
            loop {
                println!("Trying to receive event");
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
                                                &self.task_channels,
                                                &self.pending_task_creation,
                                                &self.pending_task_list,
                                            );
                                            return Some(Ok(response));
                                        }
                                        Err(e) => {
                                            error!("Failed to deserialize response: {}", e);
                                            return Some(Err(ClientError::DeserializationError(e)));
                                        }
                                    }
                                } else {
                                    debug!("Received non-text WebSocket message");
                                }
                            }
                            ewebsock::WsEvent::Error(e) => {
                                error!("WebSocket error: {}", e);
                                return Some(Err(ClientError::WebSocketError(e.to_string())));
                            }
                            ewebsock::WsEvent::Closed => {
                                debug!("WebSocket connection closed");
                                return None;
                            }
                            ewebsock::WsEvent::Opened => {
                                debug!("WebSocket connection opened");
                            }
                        }
                    }
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
        } else {
            println!("Receiver is None");
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
            task_channels: self.task_channels.clone(),
            pending_task_creation: self.pending_task_creation.clone(),
            pending_task_list: self.pending_task_list.clone(),
            state: self.state.clone(),
        }
    }
}

impl MutantClient {
    pub fn partial_take_receiver(&mut self) -> Self {
        let mut clone = self.clone();

        clone.receiver = self.receiver.take();

        clone
    }
}

// Need to run this once at the start of the WASM application
pub fn set_panic_hook() {
    console_error_panic_hook::set_once();
}
