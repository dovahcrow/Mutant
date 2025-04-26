use std::future::Future;
use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc};

use futures::channel::oneshot;
use log::{debug, error, info, warn};
use tokio::sync::mpsc;
use url::Url;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

use mutant_protocol::{
    ListTasksRequest, QueryTaskRequest, Request, Task, TaskId, TaskListEntry, TaskProgress,
    TaskResult, TaskStatus,
};

pub mod error;
mod request;
mod response;

use crate::error::ClientError;

// Shared state for tasks managed by the client (using Arc<Mutex> for thread safety)
type ClientTaskMap = Arc<Mutex<HashMap<TaskId, Task>>>;

// Remove Ws alias for web_sys::WebSocket
// type Ws = Rc<WebSocket>;

// Type alias for the pending request senders (wrapping Option in Mutex for shared access)
type PendingTaskCreationSender = Arc<Mutex<Option<PendingTaskCreation>>>;
type PendingTaskListSender =
    Arc<Mutex<Option<oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>>>>;
type PendingTaskQuerySender = Arc<Mutex<Option<oneshot::Sender<Result<Task, ClientError>>>>>;

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
    pending_task_query: PendingTaskQuerySender,
    state: Arc<Mutex<ConnectionState>>,
}

impl MutantClient {
    /// Creates a new client instance but does not connect yet.
    pub fn new() -> Self {
        Self {
            sender: None,
            receiver: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            task_channels: Arc::new(Mutex::new(HashMap::new())),
            pending_task_creation: Arc::new(Mutex::new(None)),
            pending_task_list: Arc::new(Mutex::new(None)),
            pending_task_query: Arc::new(Mutex::new(None)),
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

        *self.state.lock().unwrap() = ConnectionState::Connecting;

        let options = ewebsock::Options::default();
        let (sender, receiver) = ewebsock::connect(url.as_str(), options)
            .map_err(|e| ClientError::WebSocketError(e.to_string()))?;

        self.sender = Some(sender);
        self.receiver = Some(receiver);

        *self.state.lock().unwrap() = ConnectionState::Connected;

        let mut client_clone = self.partial_take_receiver();

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
            while let Some(response) = client_clone.next_response().await {
                if let Err(e) = response {
                    error!("Error processing response: {:?}", e);
                }
            }
        });

        Ok(())
    }

    // --- Public API Methods ---
    // A simple request/response map or channels might be needed.

    pub async fn put<'a>(
        &'a mut self,
        user_key: &str,
        source_path: &str,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + 'a,
            ProgressReceiver,
        ),
        ClientError,
    > {
        info!(
            "Starting put operation for key: {} from path: {}",
            user_key, source_path
        );

        let (completion_tx, completion_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let (task_creation_tx, task_creation_rx) = oneshot::channel();
        *self.pending_task_creation.lock().unwrap() = Some(PendingTaskCreation {
            sender: task_creation_tx,
            channels: Some((completion_tx, progress_tx)),
        });

        let req = Request::Put(mutant_protocol::PutRequest {
            user_key: user_key.to_string(),
            source_path: source_path.to_string(),
        });

        let start_task = async move {
            match self.send_request(req).await {
                Ok(_) => {
                    let task_id = task_creation_rx.await.map_err(|_| {
                        error!("TaskCreated channel canceled");
                        ClientError::InternalError("TaskCreated channel canceled".to_string())
                    })??;

                    info!("Task created with ID: {}", task_id);

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

    pub async fn get(
        &mut self,
        user_key: &str,
        destination_path: &str,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + '_,
            ProgressReceiver,
        ),
        ClientError,
    > {
        if self.pending_task_creation.lock().unwrap().is_some() {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (completion_tx, completion_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let (task_creation_tx, task_creation_rx) = oneshot::channel();
        *self.pending_task_creation.lock().unwrap() = Some(PendingTaskCreation {
            sender: task_creation_tx,
            channels: Some((completion_tx, progress_tx)),
        });

        let req = Request::Get(mutant_protocol::GetRequest {
            user_key: user_key.to_string(),
            destination_path: destination_path.to_string(),
        });

        let start_task = async move {
            match self.send_request(req).await {
                Ok(_) => {
                    debug!("Get request sent, waiting for TaskCreated response...");
                    let task_id = task_creation_rx.await.map_err(|_| {
                        ClientError::InternalError("TaskCreated channel canceled".to_string())
                    })??;

                    info!("Task created with ID: {}", task_id);

                    completion_rx.await.map_err(|_| {
                        error!("Completion channel canceled");
                        ClientError::InternalError("Completion channel canceled".to_string())
                    })?
                }
                Err(e) => {
                    *self.pending_task_creation.lock().unwrap() = None;
                    Err(e)
                }
            }
        };

        Ok((start_task, progress_rx))
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

    pub async fn query_task(&mut self, task_id: TaskId) -> Result<Task, ClientError> {
        if self.pending_task_query.lock().unwrap().is_some() {
            return Err(ClientError::InternalError(
                "Another query_task request is already pending".to_string(),
            ));
        }

        let (sender, receiver) = oneshot::channel();
        *self.pending_task_query.lock().unwrap() = Some(sender);

        let req = Request::QueryTask(QueryTaskRequest { task_id });

        match self.send_request(req).await {
            Ok(_) => {
                debug!("QueryTask request sent, waiting for response...");
                receiver.await.map_err(|_| {
                    ClientError::InternalError("TaskQuery channel canceled".to_string())
                })?
            }
            Err(e) => {
                *self.pending_task_query.lock().unwrap() = None;
                Err(e)
            }
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
            pending_task_query: self.pending_task_query.clone(),
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

pub struct PendingTaskCreation {
    pub sender: oneshot::Sender<Result<TaskId, ClientError>>,
    pub channels: Option<(CompletionSender, ProgressSender)>,
}
