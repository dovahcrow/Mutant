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
    ExportResult, HealthCheckResult, ImportResult, KeyDetails, MultipartChunkRequest,
    MultipartCompleteRequest, MultipartInitRequest, PurgeResult, PutEvent, PutSource, Request,
    Response, StatsResponse, StorageMode, SyncResult, Task, TaskId, TaskListEntry, TaskProgress,
    TaskResult, TaskStatus, TaskStoppedResponse, TaskType,
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
    Purge,
    Import,
    Export,
    HealthCheck,
    StopTask,
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
    Purge(oneshot::Sender<Result<PurgeResult, ClientError>>),
    Import(oneshot::Sender<Result<ImportResult, ClientError>>),
    Export(oneshot::Sender<Result<ExportResult, ClientError>>),
    HealthCheck(oneshot::Sender<Result<HealthCheckResult, ClientError>>),
    StopTask(oneshot::Sender<Result<TaskStoppedResponse, ClientError>>),
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
    sender: Option<nash_ws::WebSocketSender>,
    receiver: Option<nash_ws::WebSocketReceiver>,
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

        // Connect using nash-ws
        let (sender, receiver) = nash_ws::WebSocket::new(url.as_str())
            .await
            .map_err(|e| ClientError::WebSocketError(format!("{:?}", e)))?;

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
                source: PutSource::FilePath(source_path.to_string()),
                filename: Some(std::path::Path::new(source_path)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| source_path.to_string())),
                mode,
                public,
                no_verify,
            }
        )
    }

    /// Put operation with direct byte data instead of a file path
    pub async fn put_bytes<'a>(
        &'a mut self,
        user_key: &str,
        data: Vec<u8>,
        filename: Option<String>,
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
                source: PutSource::Bytes(data),
                filename,
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

    pub async fn purge(
        &mut self,
        aggressive: bool,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + '_,
            ProgressReceiver,
        ),
        ClientError,
    > {
        long_request!(self, Purge, PurgeRequest { aggressive })
    }

    pub async fn health_check(
        &mut self,
        key_name: &str,
        recycle: bool,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + '_,
            ProgressReceiver,
        ),
        ClientError,
    > {
        long_request!(
            self,
            HealthCheck,
            HealthCheckRequest {
                key_name: key_name.to_string(),
                recycle,
            }
        )
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

    pub async fn import(&mut self, file_path: &str) -> Result<ImportResult, ClientError> {
        direct_request!(
            self,
            Import,
            ImportRequest {
                file_path: file_path.to_string()
            }
        )
    }

    pub async fn export(&mut self, destination_path: &str) -> Result<ExportResult, ClientError> {
        direct_request!(
            self,
            Export,
            ExportRequest {
                destination_path: destination_path.to_string()
            }
        )
    }

    /// Stops a running task on the daemon.
    pub async fn stop_task(&mut self, task_id: TaskId) -> Result<TaskStoppedResponse, ClientError> {
        direct_request!(self, StopTask, StopTaskRequest { task_id })
    }

    /// Put operation with multipart upload for large files
    /// This method will chunk the data and send it in multiple requests
    /// to avoid blocking the WebSocket with a single large message
    pub async fn put_bytes_multipart<'a>(
        &'a mut self,
        user_key: &str,
        data: Vec<u8>,
        filename: Option<String>,
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        chunk_size: usize,
    ) -> Result<
        (
            impl Future<Output = Result<TaskResult, ClientError>> + 'a,
            ProgressReceiver,
        ),
        ClientError,
    > {
        // Create progress channel
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let progress_tx_clone = progress_tx.clone();

        // Step 1: Initialize multipart upload
        let init_request = Request::MultipartInit(MultipartInitRequest {
            user_key: user_key.to_string(),
            total_size: data.len(),
            filename,
            mode,
            public,
            no_verify,
        });

        self.send_request(init_request).await?;

        // Wait for the MultipartInit response
        let task_id = loop {
            if let Some(Ok(Response::MultipartInit(init_response))) = self.next_response().await {
                break init_response.task_id;
            }

            // Check for error responses
            if let Some(Ok(Response::Error(error))) = self.next_response().await {
                return Err(ClientError::DaemonError(error.error));
            }

            // Small delay to avoid busy waiting
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            #[cfg(target_arch = "wasm32")]
            {
                // In WASM, we need a different approach to delay
                let (tx, rx) = futures::channel::oneshot::channel();
                let _ = tx.send(());
                let _ = rx.await;
            }
        };

        // Step 2: Send chunks in separate requests
        // Calculate total chunks for logging purposes
        let _total_chunks = (data.len() + chunk_size - 1) / chunk_size;
        let data_arc = Arc::new(data);

        // Create a completion channel
        let (completion_tx, completion_rx) = oneshot::channel();

        // Instead of spawning a separate task, we'll handle the upload in this method
        // This avoids issues with MutexGuard not being Send
        let result = Self::upload_chunks(
            task_id,
            data_arc,
            chunk_size,
            progress_tx_clone,
            self,
        ).await;

        // Send the result to the completion channel
        let _ = completion_tx.send(result);

        // Return a future that will resolve when the upload is complete
        let completion_future = async move {
            match completion_rx.await {
                Ok(result) => result,
                Err(_) => Err(ClientError::ChannelClosed),
            }
        };

        Ok((completion_future, progress_rx))
    }

    /// Helper method to upload chunks
    async fn upload_chunks(
        task_id: TaskId,
        data: Arc<Vec<u8>>,
        chunk_size: usize,
        progress_tx: mpsc::UnboundedSender<Result<TaskProgress, ClientError>>,
        client: &mut MutantClient,
    ) -> Result<TaskResult, ClientError> {
        let total_size = data.len();
        let total_chunks = (total_size + chunk_size - 1) / chunk_size;

        let mut bytes_uploaded = 0;

        // Send each chunk
        for chunk_index in 0..total_chunks {
            let start = chunk_index * chunk_size;
            let end = std::cmp::min(start + chunk_size, total_size);
            let chunk_data = data[start..end].to_vec();

            // Create chunk request
            let chunk_request = Request::MultipartChunk(MultipartChunkRequest {
                task_id,
                chunk_index,
                data: chunk_data,
            });

            // Send the chunk using the client's send_request method
            client.send_request(chunk_request).await?;

            // Update progress
            bytes_uploaded += end - start;
            let _ = progress_tx.send(Ok(TaskProgress::Put(PutEvent::MultipartUploadProgress {
                bytes_uploaded,
                total_bytes: total_size,
            })));

            // Small delay to avoid overwhelming the connection
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;

            #[cfg(target_arch = "wasm32")]
            {
                // In WASM, we need a different approach to delay
                let (tx, rx) = futures::channel::oneshot::channel();
                let _ = tx.send(());
                let _ = rx.await;
            }
        }

        // Step 3: Complete the multipart upload
        let complete_request = Request::MultipartComplete(MultipartCompleteRequest {
            task_id,
        });

        // Send the complete request
        client.send_request(complete_request).await?;

        // Return a pending result - the actual result will come through the task update mechanism
        Ok(TaskResult::Pending)
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

    /// Create a new client with a WebSocket connection to the given URL
    /// This is useful for background tasks that need their own connection
    pub async fn new_with_connection(url: &str) -> Result<Self, ClientError> {
        let mut client = Self::new();
        client.connect(url).await?;
        Ok(client)
    }
}

// Need to run this once at the start of the WASM application
pub fn set_panic_hook() {
    console_error_panic_hook::set_once();
}
