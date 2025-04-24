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

pub mod error;
pub use error::ClientError;

// Shared state for tasks managed by the client (using Rc/RefCell for single-threaded WASM)
type ClientTaskMap = Rc<RefCell<HashMap<TaskId, Task>>>;

// Type alias for the WebSocket, wrapped for sharing via Rc
type Ws = Rc<WebSocket>;

/// A client for interacting with the Mutant Daemon over WebSocket (WASM implementation).
// #[derive(Clone)] // Clone might be tricky with Rc<RefCell> and callbacks
pub struct MutantClient {
    ws: Option<Ws>,
    tasks: ClientTaskMap,
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

        let tasks_clone_message = self.tasks.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            match e.data() {
                data if data.is_string() => {
                    let text = data.as_string().unwrap();
                    match serde_json::from_str::<Response>(&text) {
                        Ok(response) => {
                            Self::process_response(response, tasks_clone_message.clone());
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

        let tasks_clone_error = self.tasks.clone();
        // let state_clone_error = self.connection_state.clone();
        let onerror_callback = Closure::wrap(Box::new(move |e: ErrorEvent| {
            error!("WebSocket error: {:?}", e);
            // *state_clone_error.borrow_mut() = ConnectionState::Error(e.message());
        }) as Box<dyn FnMut(ErrorEvent)>);

        let tasks_clone_close = self.tasks.clone();
        // let state_clone_close = self.connection_state.clone();
        let onclose_callback = Closure::wrap(Box::new(move |e: CloseEvent| {
            info!(
                "WebSocket closed: code={}, reason='{}', wasClean={}",
                e.code(),
                e.reason(),
                e.was_clean()
            );
            // *state_clone_close.borrow_mut() = ConnectionState::Closed;
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
    fn process_response(response: Response, tasks: ClientTaskMap) {
        debug!("Processing server response: {:?}", response);
        let mut tasks_guard = tasks.borrow_mut(); // Use borrow_mut with Rc<RefCell>
        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                // We don't have the original request context here easily.
                // The request sender method needs to handle this.
                warn!(
                    "Received TaskCreated ({}) unexpectedly in global message handler.",
                    task_id
                );
            }
            Response::TaskUpdate(TaskUpdateResponse {
                task_id,
                status,
                progress,
            }) => {
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
            Response::TaskResult(TaskResultResponse {
                task_id,
                status,
                result,
            }) => {
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
            Response::TaskList(TaskListResponse { tasks: task_list }) => {
                // This needs a dedicated request/response mechanism, the global handler can't easily return this.
                info!("Received task list ({} tasks)", task_list.len());
            }
            Response::Error(ErrorResponse {
                error,
                original_request,
            }) => {
                // This needs correlation to the original request.
                error!(
                    "Received server error: {}. Original request: {:?}",
                    error, original_request
                );
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
        // 1. Create PutRequest
        let data_b64 = BASE64_STANDARD.encode(data);
        let req = Request::Put(mutant_protocol::PutRequest {
            user_key: user_key.to_string(),
            data_b64,
        });

        // TODO: This doesn't work - send_request is fire-and-forget.
        // The response (TaskCreated) comes back to the general onmessage handler.
        // We need a way to correlate the request to the specific response.
        // Simple approach: Generate TaskId client-side (not ideal as server confirms creation)
        // Better approach: Use a temporary listener/channel for the specific response.
        self.send_request(req).await?;

        // Placeholder: This needs a proper mechanism to wait for TaskCreatedResponse
        // maybe polling the task map after sending, or using a channel.
        // For now, we'll just return an error.
        Err(ClientError::InternalError(
            "Put response handling not implemented".to_string(),
        ))

        // Example with client-side ID (BAD - doesn't guarantee server task creation)
        // let task_id = Uuid::new_v4();
        // let req = Request::Put(...); // Include task_id in request? Protocol needs change.
        // self.send_request(req).await?;
        // Ok(task_id)
    }

    pub async fn get(&mut self, user_key: &str) -> Result<TaskId, ClientError> {
        let req = Request::Get(mutant_protocol::GetRequest {
            user_key: user_key.to_string(),
        });
        self.send_request(req).await?;
        // Similar problem as put - need to wait for TaskCreatedResponse
        Err(ClientError::InternalError(
            "Get response handling not implemented".to_string(),
        ))
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskListEntry>, ClientError> {
        let req = Request::ListTasks(ListTasksRequest);
        self.send_request(req).await?;
        // Needs a mechanism to wait for the TaskListResponse
        Err(ClientError::InternalError(
            "ListTasks response handling not implemented".to_string(),
        ))
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
