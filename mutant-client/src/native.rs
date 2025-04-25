// --- Native Specific Imports & Types ---

// Bring items from the parent module (lib.rs) into scope
use super::*;

// Bring items from mutant_protocol crate directly needed here
use mutant_protocol::{Response, Task, TaskId, TaskListEntry, TaskStatus, TaskType};

// Other necessary imports for native implementation
use crate::error::ClientError;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::{net::TcpStream, sync::oneshot};
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream,
    WebSocketStream,
};
use tracing::{debug, error, info, warn};
use url::Url;

// --- Type Aliases ---
pub(crate) type TokioMutex<T> = tokio::sync::Mutex<T>;
pub(crate) type TokioRwLock<T> = tokio::sync::RwLock<T>;

// Native WebSocket stream type, use aliased TungsteniteMessage
pub(crate) type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub(crate) type WsSink = SplitSink<WsStream, TungsteniteMessage>;
pub(crate) type WsSource = SplitStream<WsStream>;

// Task map and pending sender types using Tokio sync primitives
pub(crate) type ClientTaskMap = Arc<TokioRwLock<HashMap<TaskId, Task>>>;
pub(crate) type PendingTaskCreationSender =
    Arc<TokioMutex<Option<oneshot::Sender<Result<TaskId, ClientError>>>>>;
pub(crate) type PendingTaskListSender =
    Arc<TokioMutex<Option<oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>>>>;
pub(crate) type PendingPingSender =
    Arc<TokioMutex<Option<oneshot::Sender<Result<(), ClientError>>>>>;
pub(crate) type PendingStopSender =
    Arc<TokioMutex<Option<oneshot::Sender<Result<(), ClientError>>>>>;

// --- Native Implementation ---

// This impl block is conditionally compiled via #[cfg] in lib.rs
impl MutantClient {
    // Note: `new` is part of the native impl, but it initializes fields
    // that are also present in the WASM version (like `tasks`).
    // The `MutantClient` struct definition itself is in lib.rs.
    pub fn new() -> Self {
        // Initialize common fields with native-specific types
        Self {
            ws_sink: None,
            connection_handle: None,
            tasks: Arc::new(TokioRwLock::new(HashMap::new())),
            pending_task_creation: Arc::new(TokioMutex::new(None)),
            pending_task_list: Arc::new(TokioMutex::new(None)),
            pending_ping: Arc::new(TokioMutex::new(None)),
            pending_stop_daemon: Arc::new(TokioMutex::new(None)),
            // Wasm specific fields are not initialized here, they are conditionally compiled in the struct definition
            #[cfg(target_arch = "wasm32")]
            ws: None,
            #[cfg(target_arch = "wasm32")]
            _onopen_callback: None,
            #[cfg(target_arch = "wasm32")]
            _onmessage_callback: None,
            #[cfg(target_arch = "wasm32")]
            _onerror_callback: None,
            #[cfg(target_arch = "wasm32")]
            _onclose_callback: None,
        }
    }

    pub async fn connect(&mut self, addr: &str) -> Result<(), ClientError> {
        if self.connection_handle.is_some() {
            warn!("Already connected or connecting.");
            return Ok(());
        }

        let url = Url::parse(addr)?;
        info!("Connecting to Mutant Daemon at {}", url);

        let (ws_stream, response) = connect_async(url).await?;
        debug!(
            "WebSocket handshake completed, status: {}",
            response.status()
        );

        let (sink, source) = ws_stream.split();
        let sink = Arc::new(TokioMutex::new(sink));

        self.ws_sink = Some(sink.clone());

        // Clone necessary Arcs for the reader task
        let tasks = self.tasks.clone();
        let pending_task_creation = self.pending_task_creation.clone();
        let pending_task_list = self.pending_task_list.clone();
        let pending_ping = self.pending_ping.clone();
        let pending_stop_daemon = self.pending_stop_daemon.clone();

        // Spawn the reader task
        let handle = tokio::spawn(async move {
            Self::connection_loop(
                source,
                tasks,
                pending_task_creation,
                pending_task_list,
                pending_ping,
                pending_stop_daemon,
            )
            .await;
        });

        self.connection_handle = Some(handle);

        Ok(())
    }

    // Separate function to handle the connection reading loop
    async fn connection_loop(
        mut source: WsSource,                             // Use type alias directly
        tasks: ClientTaskMap,                             // Use type alias directly
        pending_task_creation: PendingTaskCreationSender, // Use type alias directly
        pending_task_list: PendingTaskListSender,         // Use type alias directly
        pending_ping: PendingPingSender,                  // Use type alias directly
        pending_stop_daemon: PendingStopSender,           // Use type alias directly
    ) {
        info!("WebSocket reader task started.");
        while let Some(msg_result) = source.next().await {
            match msg_result {
                Ok(msg) => {
                    if msg.is_text() {
                        match msg.to_text() {
                            Ok(text) => match serde_json::from_str::<Response>(text) {
                                Ok(response) => {
                                    // Note: process_response is defined in this impl block below
                                    Self::process_response(
                                        response,
                                        &tasks,
                                        &pending_task_creation,
                                        &pending_task_list,
                                        &pending_ping,
                                        &pending_stop_daemon,
                                    )
                                    .await;
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to deserialize server message: {}. Text: {}",
                                        e, text
                                    );
                                }
                            },
                            Err(_) => {
                                warn!("Received non-UTF8 text message");
                            }
                        }
                    } else if msg.is_close() {
                        info!("WebSocket connection closed by server.");
                        break;
                    } else if msg.is_ping() {
                        debug!("Received WebSocket ping");
                    } else if msg.is_pong() {
                        debug!("Received WebSocket pong");
                    } else if msg.is_binary() {
                        warn!("Received unexpected binary message.");
                    }
                }
                Err(e) => {
                    error!("WebSocket read error: {}", e);
                    break;
                }
            }
        }
        info!("WebSocket reader task finished.");
        // Clear pending requests when connection closes
        *pending_task_creation.lock().await = None;
        *pending_task_list.lock().await = None;
        *pending_ping.lock().await = None;
        *pending_stop_daemon.lock().await = None;
    }

    // Native implementation of processing responses
    async fn process_response(
        response: Response,
        tasks: &ClientTaskMap, // Use type alias directly
        pending_task_creation: &PendingTaskCreationSender, // Use type alias directly
        pending_task_list: &PendingTaskListSender, // Use type alias directly
        pending_ping: &PendingPingSender, // Use type alias directly
        pending_stop_daemon: &PendingStopSender, // Use type alias directly
    ) {
        debug!("Processing server response (native): {:?}", response);

        match response {
            Response::TaskCreated(res) => {
                if let Some(sender) = pending_task_creation.lock().await.take() {
                    debug!(
                        "Native: Received expected TaskCreated response for TaskId: {}",
                        res.task_id
                    );
                    if sender.send(Ok(res.task_id)).is_err() {
                        warn!("Native: Failed to send TaskCreated response (receiver dropped?)");
                    }
                } else {
                    warn!(
                        "Native: Received TaskCreated ({}) but no request was pending.",
                        res.task_id
                    );
                }
            }
            Response::TaskUpdate(res) => {
                let mut tasks_guard = tasks.write().await;
                if let Some(task) = tasks_guard.get_mut(&res.task_id) {
                    task.status = res.status;
                    task.progress = res.progress;
                    info!("Native: Task {} updated: {:?}", res.task_id, task.status);
                } else {
                    warn!("Native: Received update for unknown task: {}", res.task_id);
                    // Optionally add placeholder if needed
                    // tasks_guard.insert(...);
                }
            }
            Response::TaskResult(res) => {
                let mut tasks_guard = tasks.write().await;
                if let Some(task) = tasks_guard.get_mut(&res.task_id) {
                    task.status = res.status;
                    task.result = res.result;
                    info!("Native: Task {} finished: {:?}", res.task_id, task.status);
                } else {
                    warn!("Native: Received result for unknown task: {}", res.task_id);
                    // Optionally add placeholder if needed
                    // tasks_guard.insert(...);
                }
            }
            Response::TaskList(res) => {
                if let Some(sender) = pending_task_list.lock().await.take() {
                    debug!(
                        "Native: Received expected TaskList response ({} tasks)",
                        res.tasks.len()
                    );
                    if sender.send(Ok(res.tasks)).is_err() {
                        warn!("Native: Failed to send TaskList response (receiver dropped?)");
                    }
                } else {
                    warn!("Native: Received TaskList but no request was pending.");
                }
            }
            Response::Ls(res) => {
                warn!("Native: Received unexpected Ls response: {:?}", res);
            }
            Response::Stats(res) => {
                warn!("Native: Received unexpected Stats response: {:?}", res);
            }
            Response::TaskCancelled(res) => {
                warn!(
                    "Native: Received TaskCancelled response (not fully handled): {:?}",
                    res
                );
            }
            Response::StopDaemon(res) => {
                if let Some(sender) = pending_stop_daemon.lock().await.take() {
                    debug!("Native: Received StopDaemon response: {}", res.message);
                    if res.success {
                        if sender.send(Ok(())).is_err() {
                            warn!(
                                "Native: Failed to send StopDaemon Ok response (receiver dropped?)"
                            );
                        }
                    } else {
                        let err =
                            ClientError::ServerError(format!("StopDaemon failed: {}", res.message));
                        if sender.send(Err(err)).is_err() {
                            warn!("Native: Failed to send StopDaemon Error response (receiver dropped?)");
                        }
                    }
                } else {
                    warn!("Native: Received StopDaemon response but no request was pending.");
                }
            }
            Response::Ping(res) => {
                if let Some(sender) = pending_ping.lock().await.take() {
                    debug!("Native: Received Ping response (Pong): {}", res.message);
                    if sender.send(Ok(())).is_err() {
                        warn!("Native: Failed to send Ping Ok response (receiver dropped?)");
                    }
                } else {
                    warn!("Native: Received Ping response but no request was pending.");
                }
            }
            Response::Error(err_resp) => {
                error!("Native: Received error response: {}", err_resp.error);
                let err_string = ClientError::ServerError(err_resp.error).to_string();

                if let Some(sender) = pending_task_creation.lock().await.take() {
                    let _ = sender.send(Err(ClientError::ServerError(err_string.clone())));
                } else if let Some(sender) = pending_task_list.lock().await.take() {
                    let _ = sender.send(Err(ClientError::ServerError(err_string.clone())));
                } else if let Some(sender) = pending_ping.lock().await.take() {
                    let _ = sender.send(Err(ClientError::ServerError(err_string.clone())));
                } else if let Some(sender) = pending_stop_daemon.lock().await.take() {
                    let _ = sender.send(Err(ClientError::ServerError(err_string.clone())));
                } else {
                    warn!(
                        "Received error response '{}' but no matching specific pending sender found.",
                        err_string
                    );
                }
            }
        }
    }

    // Native-specific close logic
    // Note: This was previously in the common impl block, but it relies on native-specific fields (`connection_handle`, `ws_sink`).
    pub async fn close(&mut self) {
        info!("Closing WebSocket connection (Native)");
        if let Some(handle) = self.connection_handle.take() {
            handle.abort();
            // Maybe wait? Maybe send close frame via sink?
            // For now, just abort the reader task.
        }
        self.ws_sink = None; // Drop the sink reference

        // Clear pending requests
        *self.pending_task_creation.lock().await = None;
        *self.pending_task_list.lock().await = None;
        *self.pending_ping.lock().await = None;
        *self.pending_stop_daemon.lock().await = None;
    }
}
