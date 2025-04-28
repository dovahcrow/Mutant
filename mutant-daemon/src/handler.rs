use std::sync::Arc;
use tokio::fs; // Added for async file operations

use futures_util::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use mutant_lib::storage::{IndexEntry, PadStatus, ScratchpadAddress};
use mutant_lib::MutAnt;
use tokio::sync::mpsc;
use uuid::Uuid;
use warp::ws::{Message, WebSocket};

use crate::{error::Error as DaemonError, TaskMap};
use mutant_protocol::{
    ErrorResponse, GetCallback, GetEvent, GetRequest, KeyDetails, ListKeysRequest,
    ListKeysResponse, ListTasksRequest, PutCallback, PutEvent, PutRequest, QueryTaskRequest,
    Request, Response, RmRequest, RmSuccessResponse, StatsRequest, StatsResponse, SyncCallback,
    SyncEvent, SyncRequest, Task, TaskCreatedResponse, TaskListEntry, TaskListResponse,
    TaskProgress, TaskResult, TaskResultResponse, TaskResultType, TaskStatus, TaskType,
    TaskUpdateResponse,
};

// Helper function to send JSON responses
async fn send_response(
    sender: &mut SplitSink<WebSocket, Message>,
    response: Response,
) -> Result<(), DaemonError> {
    let json = serde_json::to_string(&response).map_err(DaemonError::SerdeJson)?;
    sender
        .send(Message::text(json))
        .await
        .map_err(DaemonError::WebSocket)?;
    Ok(())
}

pub async fn handle_ws(ws: WebSocket, mutant: Arc<MutAnt>, tasks: TaskMap) {
    let (mut ws_sender, mut ws_receiver) = ws.split();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<Response>();

    tracing::info!("WebSocket client connected");

    // Task to listen for updates from spawned tasks and send them to the client
    let update_forwarder = tokio::spawn(async move {
        while let Some(response) = update_rx.recv().await {
            if let Err(e) = send_response(&mut ws_sender, response).await {
                tracing::error!("Failed to send task update via WebSocket: {}", e);
                // If sending fails, the client might be disconnected, so we stop.
                break;
            }
        }
        // Ensure the sender is closed if the loop exits
        let _ = ws_sender.close().await;
        tracing::debug!("Update forwarder task finished.");
    });

    while let Some(result) = ws_receiver.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("WebSocket receive error: {}", e);
                // Don't need to send error here, forwarder handles closure
                break;
            }
        };

        if msg.is_close() {
            tracing::info!("WebSocket client disconnected explicitly");
            break;
        }

        if let Ok(text) = msg.to_str() {
            let original_request = text.to_string(); // Keep for error reporting
            match serde_json::from_str::<Request>(text) {
                Ok(request) => {
                    if let Err(e) = handle_request(
                        request,
                        original_request.as_str(),
                        update_tx.clone(), // Pass the update channel sender
                        mutant.clone(),
                        tasks.clone(),
                    )
                    .await
                    {
                        tracing::error!("Error handling request: {}", e);
                        let _ = update_tx.send(Response::Error(ErrorResponse {
                            error: e.to_string(),
                            original_request: Some(original_request),
                        }));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize request: {}", e);
                    let _ = update_tx.send(Response::Error(ErrorResponse {
                        error: format!("Invalid JSON request: {}", e),
                        original_request: Some(original_request),
                    }));
                }
            }
        } else if msg.is_binary() {
            tracing::warn!("Received binary message, ignoring.");
            let _ = update_tx.send(Response::Error(ErrorResponse {
                error: "Binary messages are not supported".to_string(),
                original_request: None,
            }));
        } else if msg.is_ping() {
            tracing::trace!("Received Ping");
        } else if msg.is_pong() {
            tracing::trace!("Received Pong");
        }
    }

    // Ensure the forwarder task is cleaned up when the receive loop ends
    update_forwarder.abort();
    tracing::debug!("WebSocket connection handler finished.");
}

async fn handle_request(
    request: Request,
    original_request_str: &str,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    match request {
        Request::Put(put_req) => handle_put(put_req, update_tx, mutant, tasks).await?,
        Request::Get(get_req) => handle_get(get_req, update_tx, mutant, tasks).await?,
        Request::QueryTask(query_req) => {
            handle_query_task(query_req, update_tx, tasks, original_request_str).await?
        }
        Request::ListTasks(list_req) => handle_list_tasks(list_req, update_tx, tasks).await?,
        Request::Rm(rm_req) => handle_rm(rm_req, update_tx, mutant, original_request_str).await?,
        Request::ListKeys(list_keys_req) => {
            handle_list_keys(list_keys_req, update_tx, mutant).await?
        }
        Request::Stats(stats_req) => handle_stats(stats_req, update_tx, mutant).await?,
        Request::Sync(sync_req) => handle_sync(sync_req, update_tx, mutant, tasks).await?,
    }
    Ok(())
}

async fn handle_put(
    req: PutRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();
    let source_path = req.source_path.clone(); // Keep path for logging

    // Read data from the local source path instead of decoding base64
    // let data_bytes = BASE64_STANDARD
    //     .decode(&req.data_b64)
    //     .map_err(DaemonError::Base64Decode)?;
    let data_bytes = fs::read(&req.source_path).await.map_err(|e| {
        DaemonError::IoError(format!(
            "Failed to read source file {}: {}",
            req.source_path, e
        ))
    })?;

    let task = Task {
        id: task_id,
        task_type: TaskType::Put,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
    };
    {
        tasks.write().await.insert(task_id, task.clone());
    }

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tokio::spawn(async move {
        tracing::info!(task_id = %task_id, user_key = %user_key, source_path = %source_path, "Starting PUT task");

        let mut tasks_guard = tasks.write().await;
        if let Some(task) = tasks_guard.get_mut(&task_id) {
            task.status = TaskStatus::InProgress;
        }
        drop(tasks_guard);

        // Create a callback that forwards progress events through the WebSocket
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: PutCallback = Arc::new(move |event: PutEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Put(event);

                // Update the task's progress in the map
                let mut tasks_guard = tasks.write().await;
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.progress = Some(progress.clone());
                }
                drop(tasks_guard);

                let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                    task_id,
                    status: TaskStatus::InProgress,
                    progress: Some(progress),
                }));

                Ok(true)
            })
        });

        // Set the callback on the mutant instance
        let mut mutant = (*mutant).clone();
        mutant.set_put_callback(callback).await;

        // Use the data read from the file
        let result = mutant
            .put(&user_key, &data_bytes, req.mode, req.public, req.no_verify)
            .await;

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                match result {
                    Ok(_addr) => {
                        task.status = TaskStatus::Completed;
                        task.result = TaskResult::Result(TaskResultType::Put(()));
                        tracing::info!(task_id = %task_id, user_key = %user_key, source_path = %source_path, "PUT task completed successfully");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Completed,
                            result: task.result.clone(),
                        }))
                    }
                    Err(e) => {
                        task.status = TaskStatus::Failed;
                        task.result = TaskResult::Error(e.to_string());
                        tracing::error!(task_id = %task_id, user_key = %user_key, source_path = %source_path, error = %e, "PUT task failed");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Failed,
                            result: task.result.clone(),
                        }))
                    }
                }
            } else {
                None
            }
        };

        if let Some(response) = final_response {
            let _ = update_tx.send(response);
        }
    });

    Ok(())
}

async fn handle_get(
    req: GetRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();
    let destination_path = req.destination_path.clone(); // Keep path for logging and writing

    let task = Task {
        id: task_id,
        task_type: TaskType::Get,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
    };
    {
        tasks.write().await.insert(task_id, task.clone());
    }

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tokio::spawn(async move {
        tracing::info!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, "Starting GET task");

        let mut tasks_guard = tasks.write().await;
        if let Some(task) = tasks_guard.get_mut(&task_id) {
            task.status = TaskStatus::InProgress;
        }
        drop(tasks_guard);

        // Create a callback that forwards progress events through the WebSocket
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: GetCallback = Arc::new(move |event: GetEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Get(event);

                // Update the task's progress in the map
                let mut tasks_guard = tasks.write().await;
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.progress = Some(progress.clone());
                }
                drop(tasks_guard);

                let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                    task_id,
                    status: TaskStatus::InProgress,
                    progress: Some(progress),
                }));

                Ok(true)
            })
        });

        let mut mutant = (*mutant).clone();
        mutant.set_get_callback(callback).await;

        let get_result = if req.public {
            let address = ScratchpadAddress::from_hex(&user_key).unwrap();
            mutant.get_public(&address).await
        } else {
            mutant.get(&user_key).await
        };

        let write_result = match get_result {
            Ok(data_bytes) => {
                // Write the received bytes to the destination path
                fs::write(&destination_path, &data_bytes)
                    .await
                    .map_err(|e| {
                        DaemonError::IoError(format!(
                            "Failed to write to destination file {}: {}",
                            destination_path, e
                        ))
                    })
                    .map(|_| data_bytes) // Pass data_bytes through on success for tracing length maybe
            }
            Err(e) => Err(DaemonError::LibError(e)), // Propagate the lib error
        };

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                match write_result {
                    Ok(data_bytes) => {
                        task.status = TaskStatus::Completed;
                        task.result = TaskResult::Result(TaskResultType::Get(()));
                        tracing::info!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, bytes_written = data_bytes.len(), "GET task completed successfully");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Completed,
                            result: task.result.clone(),
                        }))
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        task.status = TaskStatus::Failed;
                        task.result = TaskResult::Error(error_msg.clone());
                        tracing::error!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, "GET task failed: {}", error_msg);
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Failed,
                            result: task.result.clone(),
                        }))
                    }
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task removed before GET completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final GET result sent");
            }
        }
    });

    Ok(())
}

async fn handle_sync(
    req: SyncRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();

    let task = Task {
        id: task_id,
        task_type: TaskType::Sync,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
    };
    {
        tasks.write().await.insert(task_id, task.clone());
    }

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tokio::spawn(async move {
        tracing::info!(task_id = %task_id, "Starting SYNC task");

        let mut tasks_guard = tasks.write().await;
        if let Some(task) = tasks_guard.get_mut(&task_id) {
            task.status = TaskStatus::InProgress;
        }
        drop(tasks_guard);

        // Create a callback that forwards progress events through the WebSocket
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: SyncCallback = Arc::new(move |event: SyncEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Sync(event);

                // Update the task's progress in the map
                let mut tasks_guard = tasks.write().await;
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.progress = Some(progress.clone());
                }
                drop(tasks_guard);

                let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                    task_id,
                    status: TaskStatus::InProgress,
                    progress: Some(progress),
                }));

                Ok(true)
            })
        });

        let mut mutant = (*mutant).clone();
        mutant.set_sync_callback(callback).await;

        let sync_result = mutant.sync(req.push_force).await;

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                match sync_result {
                    Ok(sync_result) => {
                        task.status = TaskStatus::Completed;
                        task.result = TaskResult::Result(TaskResultType::Sync(sync_result));
                        tracing::info!(task_id = %task_id, "SYNC task completed successfully");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Completed,
                            result: task.result.clone(),
                        }))
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        task.status = TaskStatus::Failed;
                        task.result = TaskResult::Error(error_msg.clone());
                        tracing::error!(task_id = %task_id, "SYNC task failed: {}", error_msg);
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Failed,
                            result: task.result.clone(),
                        }))
                    }
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task removed before SYNC completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final SYNC result sent");
            }
        }
    });

    Ok(())
}

async fn handle_query_task(
    req: QueryTaskRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    tasks: TaskMap,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    let task_id = req.task_id;
    let tasks_guard = tasks.read().await;

    let response = if let Some(task) = tasks_guard.get(&task_id) {
        tracing::debug!(task_id = %task_id, "Queried task status");
        match task.status {
            TaskStatus::Completed | TaskStatus::Failed => {
                Response::TaskResult(TaskResultResponse {
                    task_id: task.id,
                    status: task.status.clone(),
                    result: task.result.clone(),
                })
            }
            _ => Response::TaskUpdate(TaskUpdateResponse {
                task_id: task.id,
                status: task.status.clone(),
                progress: task.progress.clone(),
            }),
        }
    } else {
        tracing::warn!(task_id = %task_id, "Query for unknown task");
        Response::Error(ErrorResponse {
            error: format!("Task not found: {}", task_id),
            original_request: Some(original_request_str.to_string()),
        })
    };

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

async fn handle_list_tasks(
    _req: ListTasksRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let tasks_guard = tasks.read().await;
    let task_list: Vec<TaskListEntry> = tasks_guard
        .values()
        .map(|task| TaskListEntry {
            task_id: task.id,
            task_type: task.task_type.clone(),
            status: task.status.clone(),
        })
        .collect();

    update_tx
        .send(Response::TaskList(TaskListResponse { tasks: task_list }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tracing::debug!("Listed all tasks");
    Ok(())
}

async fn handle_rm(
    req: RmRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    let user_key = req.user_key.clone();
    tracing::info!(user_key = %user_key, "Starting RM task");

    let result = mutant.rm(&user_key).await;

    let response = match result {
        Ok(_) => {
            tracing::info!(user_key = %user_key, "RM task completed successfully");
            Response::RmSuccess(RmSuccessResponse { user_key })
        }
        Err(e) => {
            tracing::error!(user_key = %user_key, error = %e, "RM task failed");
            Response::Error(ErrorResponse {
                error: e.to_string(),
                original_request: Some(original_request_str.to_string()),
            })
        }
    };

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

async fn handle_list_keys(
    _req: ListKeysRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling ListKeys request");

    // Use mutant.list() to get detailed IndexEntry data
    let index_result = mutant.list().await;

    let response = match index_result {
        Ok(index_map) => {
            tracing::info!("Found {} keys", index_map.len());
            let details: Vec<KeyDetails> = index_map
                .into_iter()
                .map(|(key, entry)| match entry {
                    IndexEntry::PrivateKey(pads) => {
                        let total_size = pads.iter().map(|p| p.size).sum::<usize>();
                        let pad_count = pads.len();
                        let confirmed_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count();
                        KeyDetails {
                            key,
                            total_size,
                            pad_count,
                            confirmed_pads,
                            is_public: false,
                            public_address: None,
                        }
                    }
                    IndexEntry::PublicUpload(index_pad, pads) => {
                        let data_size = pads.iter().map(|p| p.size).sum::<usize>();
                        let total_size = data_size + index_pad.size;
                        let pad_count = pads.len() + 1; // +1 for index pad
                        let confirmed_data_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count();
                        let index_pad_confirmed = if index_pad.status == PadStatus::Confirmed {
                            1
                        } else {
                            0
                        };
                        let confirmed_pads = confirmed_data_pads + index_pad_confirmed;
                        KeyDetails {
                            key,
                            total_size,
                            pad_count,
                            confirmed_pads,
                            is_public: true,
                            public_address: Some(index_pad.address.to_hex()),
                        }
                    }
                })
                .collect();

            Response::ListKeys(ListKeysResponse { keys: details })
        }
        Err(e) => {
            tracing::error!("Failed to list keys from mutant-lib: {}", e);
            Response::Error(ErrorResponse {
                error: format!("Failed to retrieve key list: {}", e),
                original_request: None, // Cannot easily get original string here yet
            })
        }
    };

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

async fn handle_stats(
    _req: StatsRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling Stats request");

    // Call the method which returns StorageStats directly
    let stats = mutant.get_storage_stats().await;
    tracing::info!("Retrieved storage stats successfully: {:?}", stats);

    // Adapt the fields from mutant_lib::StorageStats to mutant_protocol::StatsResponse
    let response = Response::Stats(StatsResponse {
        total_keys: stats.nb_keys,
        // total_size: stats.total_size, // This field does not exist in StorageStats
        total_pads: stats.total_pads,
        occupied_pads: stats.occupied_pads,
        free_pads: stats.free_pads,
        pending_verify_pads: stats.pending_verification_pads,
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}
