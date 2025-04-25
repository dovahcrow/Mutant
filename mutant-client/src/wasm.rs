// --- WASM Specific Imports & Types ---

// Bring items from the parent module (lib.rs) into scope
use super::*;

// Bring items from mutant_protocol crate directly needed here
use mutant_protocol::{Response, Task, TaskId, TaskListEntry, TaskStatus, TaskType};

// Other necessary imports for WASM implementation
use crate::error::ClientError;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};
use url::Url;
use wasm_bindgen::{prelude::*, JsCast, JsValue};
use web_sys::{CloseEvent, ErrorEvent, MessageEvent, WebSocket};

// --- Type Aliases ---

pub(crate) type Ws = Rc<WebSocket>;
pub(crate) type ClientTaskMap = Rc<RefCell<HashMap<TaskId, Task>>>;
pub(crate) type PendingTaskCreationSender = oneshot::Sender<Result<TaskId, ClientError>>;
pub(crate) type PendingTaskListSender = oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>;
pub(crate) type PendingPingSender = oneshot::Sender<Result<(), ClientError>>; // Placeholder for ping
pub(crate) type PendingStopSender = oneshot::Sender<Result<(), ClientError>>; // Placeholder for stop

// Type aliases for JS closures needed for WASM callbacks
pub(crate) type OnOpenClosure = Closure<dyn FnMut()>;
pub(crate) type OnMessageClosure = Closure<dyn FnMut(MessageEvent)>;
pub(crate) type OnErrorClosure = Closure<dyn FnMut(ErrorEvent)>;
pub(crate) type OnCloseClosure = Closure<dyn FnMut(CloseEvent)>;

// --- WASM Implementation ---

// This impl block is conditionally compiled via #[cfg] in lib.rs
impl MutantClient {
    pub fn new() -> Self {
        Self {
            ws: None,
            _onopen_callback: None,
            _onmessage_callback: None,
            _onerror_callback: None,
            _onclose_callback: None,
            tasks: Rc::new(RefCell::new(HashMap::new())),
            pending_task_creation: Rc::new(RefCell::new(None)), // Use Rc<RefCell<Option<...>>> directly
            pending_task_list: Rc::new(RefCell::new(None)), // Use Rc<RefCell<Option<...>>> directly
            pending_ping: Rc::new(RefCell::new(None)),      // Use Rc<RefCell<Option<...>>> directly
            pending_stop_daemon: Rc::new(RefCell::new(None)), // Use Rc<RefCell<Option<...>>> directly
            // Native specific fields are not initialized here
            #[cfg(not(target_arch = "wasm32"))]
            ws_sink: None,
            #[cfg(not(target_arch = "wasm32"))]
            connection_handle: None,
        }
    }

    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        if self.ws.is_some() {
            warn!("Already connected or connecting.");
            return Ok(());
        }

        let url = Url::parse(addr)?;
        info!("Connecting to Mutant Daemon at {} (WASM)", url);

        // Create WebSocket instance
        let ws = WebSocket::new(url.as_str()).map_err(|js_err| {
            ClientError::WebSocketError(format!("Failed to create WebSocket: {:?}", js_err))
        })?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let ws = Rc::new(ws);

        // Clone Rc containers for closures
        let tasks_clone_msg = self.tasks.clone();
        let pending_task_creation_clone_msg = self.pending_task_creation.clone();
        let pending_task_list_clone_msg = self.pending_task_list.clone();
        let pending_ping_clone_msg = self.pending_ping.clone();
        let pending_stop_daemon_clone_msg = self.pending_stop_daemon.clone();

        let onmessage_callback = OnMessageClosure::wrap(Box::new(move |e: MessageEvent| {
            match e.data() {
                data if data.is_string() => {
                    let text = data.as_string().unwrap();
                    match serde_json::from_str::<Response>(&text) {
                        Ok(response) => {
                            // Call the WASM-specific process_response (defined below)
                            Self::process_response(
                                response,
                                &tasks_clone_msg,
                                &pending_task_creation_clone_msg,
                                &pending_task_list_clone_msg,
                                &pending_ping_clone_msg,
                                &pending_stop_daemon_clone_msg,
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
                data if data.dyn_into::<js_sys::ArrayBuffer>().is_ok() => {
                    warn!("Received binary message (ArrayBuffer), ignoring.");
                }
                _ => {
                    warn!("Received non-text/non-ArrayBuffer message: {:?}", e.data());
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        // Clone Rcs again for error/close closures
        let pending_task_creation_clone_err = self.pending_task_creation.clone();
        let pending_task_list_clone_err = self.pending_task_list.clone();
        let pending_ping_clone_err = self.pending_ping.clone();
        let pending_stop_daemon_clone_err = self.pending_stop_daemon.clone();

        let onerror_callback = OnErrorClosure::wrap(Box::new(move |e: ErrorEvent| {
            error!("WebSocket error: {:?}", e);
            let err = ClientError::WebSocketError(format!("WebSocket error: {:?}", e));
            if let Some(sender) = pending_task_creation_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_task_list_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_ping_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_stop_daemon_clone_err.borrow_mut().take() {
                let _ = sender.send(Err(err));
            }
        }) as Box<dyn FnMut(ErrorEvent)>);

        let pending_task_creation_clone_close = self.pending_task_creation.clone();
        let pending_task_list_clone_close = self.pending_task_list.clone();
        let pending_ping_clone_close = self.pending_ping.clone();
        let pending_stop_daemon_clone_close = self.pending_stop_daemon.clone();

        let onclose_callback = OnCloseClosure::wrap(Box::new(move |e: CloseEvent| {
            info!(
                "WebSocket closed: code={}, reason='{}', wasClean={}",
                e.code(),
                e.reason(),
                e.was_clean()
            );
            let err = ClientError::WebSocketError(format!(
                "WebSocket closed: code={}, reason='{}'",
                e.code(),
                e.reason()
            ));
            if let Some(sender) = pending_task_creation_clone_close.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_task_list_clone_close.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_ping_clone_close.borrow_mut().take() {
                let _ = sender.send(Err(err.clone()));
            }
            if let Some(sender) = pending_stop_daemon_clone_close.borrow_mut().take() {
                let _ = sender.send(Err(err));
            }
        }) as Box<dyn FnMut(CloseEvent)>);

        let onopen_callback = OnOpenClosure::wrap(Box::new(move || {
            info!("WebSocket connection opened (WASM).");
        }) as Box<dyn FnMut()>);

        // Attach callbacks
        ws.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        ws.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));

        // Store WebSocket and closures
        self.ws = Some(ws);
        self._onopen_callback = Some(onopen_callback);
        self._onmessage_callback = Some(onmessage_callback);
        self._onerror_callback = Some(onerror_callback);
        self._onclose_callback = Some(onclose_callback);

        Ok(())
    }

    // WASM implementation of processing responses
    fn process_response(
        response: Response,
        tasks: &ClientTaskMap, // Use type alias directly
        pending_task_creation: &PendingTaskCreationSender, // Use type alias directly
        pending_task_list: &PendingTaskListSender, // Use type alias directly
        pending_ping: &PendingPingSender, // Use type alias directly
        pending_stop_daemon: &PendingStopSender, // Use type alias directly
    ) {
        debug!("Processing server response (WASM): {:?}", response);

        match response {
            Response::TaskCreated(res) => {
                if let Some(sender) = pending_task_creation.borrow_mut().take() {
                    debug!(
                        "WASM: Received expected TaskCreated response for TaskId: {}",
                        res.task_id
                    );
                    if sender.send(Ok(res.task_id)).is_err() {
                        warn!("WASM: Failed to send TaskCreated response (receiver dropped?)");
                    }
                } else {
                    warn!(
                        "WASM: Received TaskCreated ({}) but no request was pending.",
                        res.task_id
                    );
                }
            }
            Response::TaskUpdate(res) => {
                let mut tasks_guard = tasks.borrow_mut();
                if let Some(task) = tasks_guard.get_mut(&res.task_id) {
                    task.status = res.status;
                    task.progress = res.progress;
                    info!("WASM: Task {} updated: {:?}", res.task_id, task.status);
                } else {
                    // Insert a placeholder task if we receive an update for an unknown task
                    warn!("WASM: Received update for unknown task: {}", res.task_id);
                    tasks_guard.insert(
                        res.task_id,
                        Task {
                            id: res.task_id,
                            // We don't know the original type, maybe default or add Unknown type?
                            task_type: TaskType::Get, // Assuming TaskType::Get for now
                            status: res.status,
                            progress: res.progress,
                            result: None, // No result yet
                        },
                    );
                }
            }
            Response::TaskResult(res) => {
                let mut tasks_guard = tasks.borrow_mut();
                if let Some(task) = tasks_guard.get_mut(&res.task_id) {
                    task.status = res.status;
                    task.result = res.result;
                    info!("WASM: Task {} finished: {:?}", res.task_id, task.status);
                } else {
                    // Insert a placeholder task if we receive a result for an unknown task
                    warn!("WASM: Received result for unknown task: {}", res.task_id);
                    tasks_guard.insert(
                        res.task_id,
                        Task {
                            id: res.task_id,
                            task_type: TaskType::Get, // Assuming TaskType::Get
                            status: res.status,
                            result: res.result,
                            progress: None, // Progress might be irrelevant now
                        },
                    );
                }
            }
            Response::TaskList(res) => {
                if let Some(sender) = pending_task_list.borrow_mut().take() {
                    debug!(
                        "WASM: Received expected TaskList response ({} tasks)",
                        res.tasks.len()
                    );
                    if sender.send(Ok(res.tasks)).is_err() {
                        warn!("WASM: Failed to send TaskList response (receiver dropped?)");
                    }
                } else {
                    warn!("WASM: Received TaskList but no request was pending.");
                }
            }
            Response::Ls(res) => {
                warn!("WASM: Received unexpected Ls response: {:?}", res);
            }
            Response::Stats(res) => {
                warn!("WASM: Received unexpected Stats response: {:?}", res);
            }
            Response::TaskCancelled(res) => {
                warn!(
                    "WASM: Received TaskCancelled response (not fully handled): {:?}",
                    res
                );
            }
            Response::StopDaemon(res) => {
                if let Some(sender) = pending_stop_daemon.borrow_mut().take() {
                    debug!("WASM: Received StopDaemon response: {}", res.message);
                    if res.success {
                        if sender.send(Ok(())).is_err() {
                            warn!(
                                "WASM: Failed to send StopDaemon Ok response (receiver dropped?)"
                            );
                        }
                    } else {
                        let err =
                            ClientError::ServerError(format!("StopDaemon failed: {}", res.message));
                        if sender.send(Err(err)).is_err() {
                            warn!("WASM: Failed to send StopDaemon Error response (receiver dropped?)");
                        }
                    }
                } else {
                    warn!("WASM: Received StopDaemon response but no request was pending.");
                }
            }
            Response::Ping(res) => {
                if let Some(sender) = pending_ping.borrow_mut().take() {
                    debug!("WASM: Received Ping response (Pong): {}", res.message);
                    if sender.send(Ok(())).is_err() {
                        warn!("WASM: Failed to send Ping Ok response (receiver dropped?)");
                    }
                } else {
                    warn!("WASM: Received Ping response but no request was pending.");
                }
            }
            Response::Error(res) => {
                error!(
                    "WASM: Received server error: {}. Original request: {:?}",
                    res.error, res.original_request
                );
                let err = ClientError::ServerError(res.error);
                if let Some(sender) = pending_task_creation.borrow_mut().take() {
                    let _ = sender.send(Err(err.clone()));
                } else if let Some(sender) = pending_task_list.borrow_mut().take() {
                    let _ = sender.send(Err(err.clone()));
                } else if let Some(sender) = pending_ping.borrow_mut().take() {
                    let _ = sender.send(Err(err.clone()));
                } else if let Some(sender) = pending_stop_daemon.borrow_mut().take() {
                    let _ = sender.send(Err(err));
                }
            }
        }
    }

    // WASM-specific close logic
    // Note: This was previously in the common impl block
    pub async fn close(&mut self) {
        if let Some(ws) = self.ws.take() {
            // Detach closures first to avoid potential issues if close triggers events
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onerror(None);
            ws.set_onclose(None);
            // Attempt to close the websocket cleanly
            match ws.close() {
                Ok(_) => info!("WebSocket close initiated (WASM)"),
                Err(e) => warn!("Error initiating WebSocket close (WASM): {:?}", e),
            }
        }
        // Drop the closures by setting them to None
        self._onopen_callback = None;
        self._onmessage_callback = None;
        self._onerror_callback = None;
        self._onclose_callback = None;

        // Clear pending requests
        *self.pending_task_creation.borrow_mut() = None;
        *self.pending_task_list.borrow_mut() = None;
        *self.pending_ping.borrow_mut() = None;
        *self.pending_stop_daemon.borrow_mut() = None;
    }
}
