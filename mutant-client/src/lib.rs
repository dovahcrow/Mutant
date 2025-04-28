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
    KeyDetails, Request, StatsResponse, StorageMode, SyncResult, Task, TaskId, TaskListEntry,
    TaskProgress, TaskResult, TaskStatus, TaskType,
};

pub mod error;
mod macros;
mod request;
mod response;

use crate::error::ClientError;

// Shared state for tasks managed by the client (using Arc<Mutex> for thread safety)
type ClientTaskMap = Arc<Mutex<HashMap<TaskId, Task>>>;

// Key for the pending requests map
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PendingRequestKey {
    TaskCreation,
    ListTasks,
    QueryTask,
    Rm,
    ListKeys,
    Stats,
    Sync,
}

// Enum to hold the different sender types for the pending requests map
pub enum PendingSender {
    TaskCreation(
        oneshot::Sender<Result<TaskId, ClientError>>,
        TaskChannels,
        TaskType,
    ),
    ListTasks(oneshot::Sender<Result<Vec<TaskListEntry>, ClientError>>),
    QueryTask(oneshot::Sender<Result<Task, ClientError>>),
    Rm(oneshot::Sender<Result<(), ClientError>>),
    ListKeys(oneshot::Sender<Result<Vec<KeyDetails>, ClientError>>),
    Stats(oneshot::Sender<Result<StatsResponse, ClientError>>),
    Sync(oneshot::Sender<Result<SyncResult, ClientError>>),
}

// The new map type for pending requests
type PendingRequestMap = Arc<Mutex<HashMap<PendingRequestKey, PendingSender>>>;

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
    pending_requests: PendingRequestMap,
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
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
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
        mode: StorageMode,
        public: bool,
        no_verify: bool,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + 'a,
            ProgressReceiver,
        ),
        ClientError,
    > {
        long_request!(
            self,
            Put,
            PutRequest {
                user_key: user_key.to_string(),
                source_path: source_path.to_string(),
                mode,
                public,
                no_verify,
            }
        )
    }

    pub async fn get(
        &mut self,
        user_key: &str,
        destination_path: &str,
        public: bool,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + '_,
            ProgressReceiver,
        ),
        ClientError,
    > {
        long_request!(
            self,
            Get,
            GetRequest {
                user_key: user_key.to_string(),
                destination_path: destination_path.to_string(),
                public,
            }
        )
    }

    pub async fn sync(
        &mut self,
        push_force: bool,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + '_,
            ProgressReceiver,
        ),
        ClientError,
    > {
        long_request!(self, Sync, SyncRequest { push_force })
    }

    /// Retrieves a list of all stored keys from the daemon.
    pub async fn list_keys(&mut self) -> Result<Vec<KeyDetails>, ClientError> {
        direct_request!(self, ListKeys, ListKeysRequest)
    }

    pub async fn rm(&mut self, user_key: &str) -> Result<(), ClientError> {
        direct_request!(
            self,
            Rm,
            RmRequest {
                user_key: user_key.to_string(),
            }
        )
    }

    pub async fn list_tasks(&mut self) -> Result<Vec<TaskListEntry>, ClientError> {
        direct_request!(self, ListTasks, ListTasksRequest)
    }

    pub async fn query_task(&mut self, task_id: TaskId) -> Result<Task, ClientError> {
        direct_request!(self, QueryTask, QueryTaskRequest { task_id })
    }

    pub async fn get_stats(&mut self) -> Result<StatsResponse, ClientError> {
        direct_request!(self, Stats, StatsRequest {})
    }

    // --- Accessor methods for internal state ---

    pub fn get_task_status(&self, task_id: TaskId) -> Option<TaskStatus> {
        self.tasks
            .lock()
            .unwrap()
            .get(&task_id)
            .map(|t| t.status.clone())
    }

    pub fn get_task_result(&self, task_id: TaskId) -> Option<TaskResult> {
        self.tasks
            .lock()
            .unwrap()
            .get(&task_id)
            .map(|t| t.result.clone())
    }
}

impl Clone for MutantClient {
    fn clone(&self) -> Self {
        Self {
            sender: None,
            receiver: None,
            tasks: self.tasks.clone(),
            task_channels: self.task_channels.clone(),
            pending_requests: self.pending_requests.clone(), // Clone the new map
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
