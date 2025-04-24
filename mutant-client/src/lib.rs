// This file will contain the client library implementation

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mutant_protocol::{
    ErrorResponse, GetRequest, ListTasksRequest, PutRequest, QueryTaskRequest, Request, Response,
    Task, TaskCreatedResponse, TaskId, TaskListEntry, TaskListResponse, TaskResultResponse,
    TaskStatus, TaskType, TaskUpdateResponse,
};
// use tokio::sync::RwLock; // Removed this - using Rc<RefCell<>> for WASM

use tracing::{debug, error, info, warn};
use url::Url;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{CloseEvent, ErrorEvent, MessageEvent, WebSocket};

// Base64 engine
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use futures::channel::oneshot;

pub mod error;
pub use error::ClientError;

// Shared state for tasks managed by the client (using Rc/RefCell for single-threaded WASM)
type ClientTaskMap = Rc<RefCell<HashMap<TaskId, Task>>>;

// Type alias for the WebSocket, wrapped for sharing via Rc
type Ws = Rc<WebSocket>;

// Type alias for the pending request senders
type PendingTaskCreationSender = oneshot::Sender<Result<TaskId, ClientError>>;
type PendingTaskListSender = oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>; // Send Result

/// A client for interacting with the Mutant Daemon over WebSocket (WASM implementation).
// #[derive(Clone)] // Clone might be tricky with Rc<RefCell> and callbacks
pub struct MutantClient {
    ws: Option<Ws>,
    tasks: ClientTaskMap,
    // Senders for pending requests expecting a specific response
    pending_task_creation: Rc<RefCell<Option<PendingTaskCreationSender>>>,
    pending_task_list: Rc<RefCell<Option<PendingTaskListSender>>>,
    // Callbacks need to be stored so they aren't dropped
    _onopen_callback: Option<Closure<dyn FnMut()>>,
    _onmessage_callback: Option<Closure<dyn FnMut(MessageEvent)>>,
    _onerror_callback: Option<Closure<dyn FnMut(ErrorEvent)>>,
    _onclose_callback: Option<Closure<dyn FnMut(CloseEvent)>>,
    // Channel to signal connection status?
    // connection_state: Rc<RefCell<ConnectionState>> // Example
}

// enum ConnectionState { Connecting, Open, Closed, Error(String) }

impl MutantClient {
    /// Creates a new client instance but does not connect yet.
    pub fn new() -> Self {
        Self {
            ws: None,
            tasks: Rc::new(RefCell::new(HashMap::new())),
            pending_task_creation: Rc::new(RefCell::new(None)), // Initialize pending senders
            pending_task_list: Rc::new(RefCell::new(None)),     // Initialize pending senders
            _onopen_callback: None,
            _onmessage_callback: None,
            _onerror_callback: None,
            _onclose_callback: None,
            // connection_state: Rc::new(RefCell::new(ConnectionState::Closed)),
        }
    }

    /// Establishes a WebSocket connection to the Mutant Daemon.
    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        if self.ws.is_some() {
            warn!("Already connected or connecting.");
            return Ok(()); // Or return an error?
        }

        let url = Url::parse(addr)?;
        info!("Connecting to Mutant Daemon at {}", url);

        // Create WebSocket instance
        let ws = WebSocket::new(url.as_str())?;
        ws.set_binary_type(web_sys::BinaryType::Blob); // Or Arraybuffer
        let ws = Rc::new(ws);

        // --- Create Closures for callbacks ---
        // Need to clone Rc pointers to be moved into closures
        let tasks_clone_open = self.tasks.clone();
        // let state_clone_open = self.connection_state.clone();
        let onopen_callback = Closure::wrap(Box::new(move || {
            info!("WebSocket connection opened.");
            // *state_clone_open.borrow_mut() = ConnectionState::Open;
            // TODO: Handle potential tasks added before connection was fully open?
        }) as Box<dyn FnMut()>);

        // Clone Rc<RefCell<>> for pending senders to move into onmessage
        let tasks_clone_message = self.tasks.clone();
        let pending_task_creation_clone = self.pending_task_creation.clone();
        let pending_task_list_clone = self.pending_task_list.clone();

        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            match e.data() {
                data if data.is_string() => {
                    let text = data.as_string().unwrap();
                    match serde_json::from_str::<Response>(&text) {
                        Ok(response) => {
                            // Pass pending senders to process_response
                            Self::process_response(
                                response,
                                tasks_clone_message.clone(),
                                pending_task_creation_clone.clone(),
                                pending_task_list_clone.clone(),
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
                // Handle binary data if needed later
                // data if data.instance_of::<web_sys::Blob>().unwrap_or(false) => {
                //     warn!("Received unexpected binary message (Blob)");
                // }
                // data if data.instance_of::<js_sys::ArrayBuffer>().unwrap_or(false) => {
                //     warn!("Received unexpected binary message (ArrayBuffer)");
                // }
                _ => {
                    warn!("Received non-text message: {:?}", e.data());
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        // Clone Rc<RefCell<>> for pending senders to potentially clear them on error/close
        let pending_task_creation_clone_err = self.pending_task_creation.clone();
        let pending_task_list_clone_err = self.pending_task_list.clone();
        let onerror_callback = Closure::wrap(Box::new(move |e: ErrorEvent| {
            error!("WebSocket error: {:?}", e);
            // Signal error to pending requests
            if let Some(sender) = pending_task_creation_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(ClientError::WebSocketError(e.message())));
            }
            if let Some(sender) = pending_task_list_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(ClientError::WebSocketError(e.message())));
            }
        }) as Box<dyn FnMut(ErrorEvent)>);

        let pending_task_creation_clone_close = self.pending_task_creation.clone();
        let pending_task_list_clone_close = self.pending_task_list.clone();
        let onclose_callback = Closure::wrap(Box::new(move |e: CloseEvent| {
            info!(
                "WebSocket closed: code={}, reason='{}', wasClean={}",
                e.code(),
                e.reason(),
                e.was_clean()
            );
            // Signal close to pending requests
            let close_err = || {
                Err(ClientError::WebSocketError(format!(
                    "WebSocket closed: code={}, reason='{}'",
                    e.code(),
                    e.reason()
                )))
            };
            if let Some(sender) = pending_task_creation_clone_close.borrow_mut().take() {
                let _ = sender.send(close_err());
            }
            if let Some(sender) = pending_task_list_clone_close.borrow_mut().take() {
                let _ = sender.send(close_err());
            }
        }) as Box<dyn FnMut(CloseEvent)>);

        // Attach callbacks
        ws.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        ws.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));

        // Store closures so they stay alive
        self._onopen_callback = Some(onopen_callback);
        self._onmessage_callback = Some(onmessage_callback);
        self._onerror_callback = Some(onerror_callback);
        self._onclose_callback = Some(onclose_callback);

        self.ws = Some(ws);
        // *self.connection_state.borrow_mut() = ConnectionState::Connecting;

        // Note: Connection happens asynchronously. We return Ok here.
        // The caller should ideally wait for the onopen event or check state.
        Ok(())
    }

    /// Processes a deserialized response from the server (called from onmessage).
    /// Needs to take Rc<RefCell<...>> because it's called from a closure.
    fn process_response(
        response: Response,
        tasks: ClientTaskMap,
        pending_task_creation: Rc<RefCell<Option<PendingTaskCreationSender>>>,
        pending_task_list: Rc<RefCell<Option<PendingTaskListSender>>>,
    ) {
        debug!("Processing server response: {:?}", response);

        match response {
            Response::TaskCreated(res @ TaskCreatedResponse { task_id }) => {
                // Check if we were waiting for this
                if let Some(sender) = pending_task_creation.borrow_mut().take() {
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
                // Still update the main task map if the task already exists (e.g., from query)
                // Or should TaskCreated *only* go via oneshot?
                // Let's assume TaskCreated is *only* for the initial request.
            }
            Response::TaskUpdate(
                res @ TaskUpdateResponse {
                    task_id,
                    status,
                    progress,
                },
            ) => {
                let mut tasks_guard = tasks.borrow_mut();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.progress = progress;
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
            Response::TaskResult(
                res @ TaskResultResponse {
                    task_id,
                    status,
                    result,
                },
            ) => {
                let mut tasks_guard = tasks.borrow_mut();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.result = result;
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
            Response::TaskList(res @ TaskListResponse { tasks: task_list }) => {
                // Check if we were waiting for this
                if let Some(sender) = pending_task_list.borrow_mut().take() {
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
            Response::Error(
                res @ ErrorResponse {
                    error,
                    original_request,
                },
            ) => {
                // Check if this error corresponds to a pending request
                if let Some(sender) = pending_task_creation.borrow_mut().take() {
                    warn!(
                        "Received Error response while waiting for TaskCreated: {}",
                        error
                    );
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else if let Some(sender) = pending_task_list.borrow_mut().take() {
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
    async fn send_request(&self, request: Request) -> Result<(), ClientError> {
        if let Some(ws) = &self.ws {
            // Check connection state before sending
            match ws.ready_state() {
                WebSocket::OPEN => {
                    let json = serde_json::to_string(&request)?;
                    ws.send_with_str(&json)
                        .map_err(|js_err| ClientError::WebSocketError(format!("{:?}", js_err)))?;
                    Ok(())
                }
                WebSocket::CONNECTING => {
                    // Could wait or queue, but erroring is simpler for now
                    Err(ClientError::WebSocketError(
                        "WebSocket is still connecting".to_string(),
                    ))
                }
                _ => Err(ClientError::NotConnected), // CLOSED or CLOSING
            }
        } else {
            Err(ClientError::NotConnected)
        }
    }

    // --- Public API Methods ---
    // These need rethinking for WASM's async/callback model.
    // A simple request/response map or channels might be needed.

    pub async fn put(&mut self, user_key: &str, data: &[u8]) -> Result<TaskId, ClientError> {
        // Ensure only one put/get request is pending
        if self.pending_task_creation.borrow().is_some() {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_creation.borrow_mut() = Some(sender);

        let data_b64 = BASE64_STANDARD.encode(data);
        let req = Request::Put(mutant_protocol::PutRequest {
            user_key: user_key.to_string(),
            data_b64,
        });

        match self.send_request(req).await {
            Ok(_) => {
                debug!("Put request sent, waiting for TaskCreated response...");
                // Wait for the response from the onmessage handler via the oneshot channel
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskCreated channel canceled".to_string())
                })?
            }
            Err(e) => {
                // Request failed, clear the pending sender
                *self.pending_task_creation.borrow_mut() = None;
                Err(e)
            }
        }
    }

    pub async fn get(&mut self, user_key: &str) -> Result<TaskId, ClientError> {
        if self.pending_task_creation.borrow().is_some() {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_creation.borrow_mut() = Some(sender);

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
                *self.pending_task_creation.borrow_mut() = None;
                Err(e)
            }
        }
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskListEntry>, ClientError> {
        if self.pending_task_list.borrow().is_some() {
            return Err(ClientError::InternalError(
                "Another list_tasks request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_list.borrow_mut() = Some(sender);

        let req = Request::ListTasks(ListTasksRequest);

        match self.send_request(req).await {
            Ok(_) => {
                debug!("ListTasks request sent, waiting for TaskList response...");
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskList channel canceled".to_string())
                })?
            }
            Err(e) => {
                *self.pending_task_list.borrow_mut() = None;
                Err(e)
            }
        }
    }

    pub async fn query_task(&self, task_id: TaskId) -> Result<(), ClientError> {
        // Sends the request, result comes back to onmessage and updates the internal map.
        let req = Request::QueryTask(QueryTaskRequest { task_id });
        self.send_request(req).await
    }

    // --- Accessor methods for internal state ---

    pub fn get_task_status(&self, task_id: TaskId) -> Option<TaskStatus> {
        self.tasks.borrow().get(&task_id).map(|t| t.status.clone())
    }

    pub fn get_task_result(&self, task_id: TaskId) -> Option<mutant_protocol::TaskResult> {
        self.tasks
            .borrow()
            .get(&task_id)
            .and_then(|t| t.result.clone())
    }
}

// Need to run this once at the start of the WASM application
pub fn set_panic_hook() {
    console_error_panic_hook::set_once();
}
