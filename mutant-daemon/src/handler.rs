use std::sync::Arc;
use tokio::fs; // Added for async file operations

use futures_util::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use mutant_lib::storage::{IndexEntry, PadInfo, PadStatus, ScratchpadAddress};
use mutant_lib::MutAnt;
use tokio::sync::mpsc;
use uuid::Uuid;
use warp::ws::{Message, WebSocket};

use crate::{error::Error as DaemonError, TaskEntry, TaskMap};
use mutant_protocol::{
    ErrorResponse, ExportRequest, ExportResponse, ExportResult, GetCallback, GetEvent, GetRequest,
    GetResult, HealthCheckCallback, HealthCheckEvent, HealthCheckRequest, ImportRequest,
    ImportResponse, ImportResult, KeyDetails, ListKeysRequest, ListKeysResponse, ListTasksRequest,
    PurgeCallback, PurgeEvent, PurgeRequest, PutCallback, PutEvent, PutRequest, QueryTaskRequest,
    Request, Response, RmRequest, RmSuccessResponse, StatsRequest, StatsResponse, StopTaskRequest,
    SyncCallback, SyncEvent, SyncRequest, Task, TaskCreatedResponse, TaskListEntry,
    TaskListResponse, TaskProgress, TaskResult, TaskResultResponse, TaskResultType, TaskStatus,
    TaskStoppedResponse, TaskType, TaskUpdateResponse,
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
        Request::Purge(purge_req) => handle_purge(purge_req, update_tx, mutant, tasks).await?,
        Request::Import(import_req) => handle_import(import_req, update_tx, mutant).await?,
        Request::Export(export_req) => handle_export(export_req, update_tx, mutant).await?,
        Request::HealthCheck(health_check_req) => {
            handle_health_check(health_check_req, update_tx, mutant, tasks).await?
        }
        Request::StopTask(stop_task_req) => {
            handle_stop_task(stop_task_req, update_tx, tasks).await?
        }
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
    // We will insert the TaskEntry after spawning the task and getting the handle

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;

        tracing::info!(task_id = %task_id, user_key = %user_key, source_path = %source_path, "Starting PUT task");

        // Update status in TaskMap
        {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                entry.task.status = TaskStatus::InProgress;
            }
        } // RwLockWriteGuard is dropped here

        // Create callback *inside* the task
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: PutCallback = Arc::new(move |event: PutEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Put(event);
                // Update task progress in map
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    // Only update if the task is still considered InProgress
                    if entry.task.status == TaskStatus::InProgress {
                        entry.task.progress = Some(progress.clone());
                        // Send update via channel
                        let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                            task_id,
                            status: TaskStatus::InProgress,
                            progress: Some(progress),
                        }));
                    } else {
                        tracing::warn!(task_id = %task_id, "Received PUT progress update for task not InProgress (status: {:?}). Ignoring.", entry.task.status);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Call put with the callback
        let result = mutant
            .put(
                &user_key,
                &data_bytes,
                req.mode,
                req.public,
                req.no_verify,
                Some(callback), // Pass callback here
            )
            .await;

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            // Check if task entry still exists and hasn't been stopped
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                // Only update if the task hasn't been stopped externally
                if entry.task.status != TaskStatus::Stopped {
                    match result {
                        Ok(_addr) => {
                            entry.task.status = TaskStatus::Completed;
                            entry.task.result = TaskResult::Result(TaskResultType::Put(()));
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::info!(task_id = %task_id, user_key = %user_key, source_path = %source_path, "PUT task completed successfully");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Completed,
                                result: entry.task.result.clone(),
                            }))
                        }
                        Err(e) => {
                            entry.task.status = TaskStatus::Failed;
                            entry.task.result = TaskResult::Error(e.to_string());
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::error!(task_id = %task_id, user_key = %user_key, source_path = %source_path, error = %e, "PUT task failed");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    tracing::info!(task_id = %task_id, "PUT task was stopped before completion.");
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task entry removed before PUT completion?");
                None
            }
        };

        if let Some(response) = final_response {
            let _ = update_tx.send(response);
        }
    });

    // Get the abort handle and create the TaskEntry
    let abort_handle = task_handle.abort_handle();
    let task_entry = TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

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

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;

        tracing::info!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, "Starting GET task");

        // Update status in TaskMap
        {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                entry.task.status = TaskStatus::InProgress;
            }
        }

        // Create callback *inside* the task
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: GetCallback = Arc::new(move |event: GetEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Get(event);
                // Update task progress
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    // Only update if the task is still considered InProgress
                    if entry.task.status == TaskStatus::InProgress {
                        entry.task.progress = Some(progress.clone());
                        // Send update via channel
                        let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                            task_id,
                            status: TaskStatus::InProgress,
                            progress: Some(progress),
                        }));
                    } else {
                        tracing::warn!(task_id = %task_id, "Received GET progress update for task not InProgress (status: {:?}). Ignoring.", entry.task.status);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Call get or get_public with the callback
        let get_result = if req.public {
            // TODO: Fix public key handling if necessary, ScratchpadAddress requires valid hex
            match ScratchpadAddress::from_hex(&user_key) {
                Ok(address) => mutant.get_public(&address, Some(callback)).await,
                Err(hex_err) => {
                    // Wrap the underlying lib error in DaemonError::LibError
                    let lib_err = mutant_lib::error::Error::Internal(format!(
                        "Invalid public key hex format for '{}': {}",
                        user_key, hex_err
                    ));
                    Err(lib_err)
                }
            }
        } else {
            mutant.get(&user_key, Some(callback)).await // Pass callback
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
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                // Only update if the task hasn't been stopped externally
                if entry.task.status != TaskStatus::Stopped {
                    match write_result {
                        Ok(data_bytes) => {
                            entry.task.status = TaskStatus::Completed;
                            entry.task.result =
                                TaskResult::Result(TaskResultType::Get(GetResult {
                                    size: data_bytes.len(),
                                }));
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::info!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, bytes_written = data_bytes.len(), "GET task completed successfully");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Completed,
                                result: entry.task.result.clone(),
                            }))
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            entry.task.status = TaskStatus::Failed;
                            entry.task.result = TaskResult::Error(error_msg.clone());
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::error!(task_id = %task_id, user_key = %user_key, destination_path = %destination_path, "GET task failed: {}", error_msg);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    tracing::info!(task_id = %task_id, "GET task was stopped before completion.");
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task entry removed before GET completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final GET result sent");
            }
        }
    });

    // Get the abort handle and create the TaskEntry
    let abort_handle = task_handle.abort_handle();
    let task_entry = TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

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

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;

        tracing::info!(task_id = %task_id, "Starting SYNC task");

        // Update status in TaskMap
        {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                entry.task.status = TaskStatus::InProgress;
            }
        }

        // Create callback *inside* the task
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: SyncCallback = Arc::new(move |event: SyncEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Sync(event);
                // Update task progress
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    // Only update if the task is still considered InProgress
                    if entry.task.status == TaskStatus::InProgress {
                        entry.task.progress = Some(progress.clone());
                        // Send update via channel
                        let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                            task_id,
                            status: TaskStatus::InProgress,
                            progress: Some(progress),
                        }));
                    } else {
                        tracing::warn!(task_id = %task_id, "Received SYNC progress update for task not InProgress (status: {:?}). Ignoring.", entry.task.status);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Call sync with the callback
        let sync_result = mutant.sync(req.push_force, Some(callback)).await; // Pass callback

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                // Only update if the task hasn't been stopped externally
                if entry.task.status != TaskStatus::Stopped {
                    match sync_result {
                        Ok(sync_result_data) => {
                            entry.task.status = TaskStatus::Completed;
                            entry.task.result =
                                TaskResult::Result(TaskResultType::Sync(sync_result_data));
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::info!(task_id = %task_id, "SYNC task completed successfully");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Completed,
                                result: entry.task.result.clone(),
                            }))
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            entry.task.status = TaskStatus::Failed;
                            entry.task.result = TaskResult::Error(error_msg.clone());
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::error!(task_id = %task_id, "SYNC task failed: {}", error_msg);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    tracing::info!(task_id = %task_id, "SYNC task was stopped before completion.");
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task entry removed before SYNC completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final SYNC result sent");
            }
        }
    });

    // Get the abort handle and create the TaskEntry
    let abort_handle = task_handle.abort_handle();
    let task_entry = TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

    Ok(())
}

async fn handle_purge(
    req: PurgeRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();

    let task = Task {
        id: task_id,
        // TaskType should be Purge, not Sync
        task_type: TaskType::Purge,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
    };

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;

        tracing::info!(task_id = %task_id, "Starting PURGE task");

        // Update status in TaskMap
        {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                entry.task.status = TaskStatus::InProgress;
            }
        }

        // Create callback *inside* the task
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: PurgeCallback = Arc::new(move |event: PurgeEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::Purge(event);
                // Update task progress
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    // Only update if the task is still considered InProgress
                    if entry.task.status == TaskStatus::InProgress {
                        entry.task.progress = Some(progress.clone());
                        // Send update via channel
                        let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                            task_id,
                            status: TaskStatus::InProgress,
                            progress: Some(progress),
                        }));
                    } else {
                        tracing::warn!(task_id = %task_id, "Received PURGE progress update for task not InProgress (status: {:?}). Ignoring.", entry.task.status);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Call purge with the callback
        let purge_result = mutant.purge(req.aggressive, Some(callback)).await; // Pass callback

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                // Only update if the task hasn't been stopped externally
                if entry.task.status != TaskStatus::Stopped {
                    match purge_result {
                        Ok(purge_result_data) => {
                            entry.task.status = TaskStatus::Completed;
                            entry.task.result =
                                TaskResult::Result(TaskResultType::Purge(purge_result_data));
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::info!(task_id = %task_id, "PURGE task completed successfully");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Completed,
                                result: entry.task.result.clone(),
                            }))
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            entry.task.status = TaskStatus::Failed;
                            entry.task.result = TaskResult::Error(error_msg.clone());
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::error!(task_id = %task_id, "PURGE task failed: {}", error_msg);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    tracing::info!(task_id = %task_id, "PURGE task was stopped before completion.");
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task entry removed before PURGE completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final PURGE result sent");
            }
        }
    });

    // Get the abort handle and create the TaskEntry
    let abort_handle = task_handle.abort_handle();
    let task_entry = TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

    Ok(())
}

async fn handle_health_check(
    req: HealthCheckRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();

    let task = Task {
        id: task_id,
        task_type: TaskType::HealthCheck,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
    };

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;

        tracing::info!(task_id = %task_id, "Starting HEALTH CHECK task");

        // Update status in TaskMap
        {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                entry.task.status = TaskStatus::InProgress;
            }
        }

        // Create callback *inside* the task
        let update_tx_clone = update_tx.clone();
        let task_id_clone = task_id;
        let tasks_clone = tasks.clone();
        let callback: HealthCheckCallback = Arc::new(move |event: HealthCheckEvent| {
            let tx = update_tx_clone.clone();
            let task_id = task_id_clone;
            let tasks = tasks_clone.clone();
            Box::pin(async move {
                let progress = TaskProgress::HealthCheck(event);
                // Update task progress
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    // Only update if the task is still considered InProgress
                    if entry.task.status == TaskStatus::InProgress {
                        entry.task.progress = Some(progress.clone());
                        // Send update via channel
                        let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                            task_id,
                            status: TaskStatus::InProgress,
                            progress: Some(progress),
                        }));
                    } else {
                        tracing::warn!(task_id = %task_id, "Received HEALTH_CHECK progress update for task not InProgress (status: {:?}). Ignoring.", entry.task.status);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Call health_check with the callback
        let health_check_result = mutant
            .health_check(&req.key_name, req.recycle, Some(callback))
            .await; // Pass callback

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(entry) = tasks_guard.get_mut(&task_id) {
                // Only update if the task hasn't been stopped externally
                if entry.task.status != TaskStatus::Stopped {
                    match health_check_result {
                        Ok(health_check_result_data) => {
                            entry.task.status = TaskStatus::Completed;
                            entry.task.result = TaskResult::Result(TaskResultType::HealthCheck(
                                health_check_result_data,
                            ));
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::info!(task_id = %task_id, "HEALTH CHECK task completed successfully");
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Completed,
                                result: entry.task.result.clone(),
                            }))
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            entry.task.status = TaskStatus::Failed;
                            entry.task.result = TaskResult::Error(error_msg.clone());
                            entry.abort_handle = None; // Task finished, remove handle
                            tracing::error!(task_id = %task_id, "HEALTH CHECK task failed: {}", error_msg);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    tracing::info!(task_id = %task_id, "HEALTH CHECK task was stopped before completion.");
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                tracing::warn!(task_id = %task_id, "Task entry removed before HEALTH CHECK completion?");
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                tracing::debug!(task_id = %task_id, "Client disconnected before final HEALTH CHECK result sent");
            }
        }
    });

    // Get the abort handle and create the TaskEntry
    let abort_handle = task_handle.abort_handle();
    let task_entry = TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

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

    let response = if let Some(entry) = tasks_guard.get(&task_id) {
        tracing::debug!(task_id = %task_id, "Queried task status");
        match entry.task.status {
            TaskStatus::Completed | TaskStatus::Failed => {
                Response::TaskResult(TaskResultResponse {
                    task_id: entry.task.id,
                    status: entry.task.status.clone(),
                    result: entry.task.result.clone(),
                })
            }
            _ => Response::TaskUpdate(TaskUpdateResponse {
                task_id: entry.task.id,
                status: entry.task.status.clone(),
                progress: entry.task.progress.clone(),
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
        .map(|entry| TaskListEntry {
            task_id: entry.task.id,
            task_type: entry.task.task_type.clone(),
            status: entry.task.status.clone(),
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

async fn handle_import(
    req: ImportRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling Import request");

    let file_path = req.file_path.clone();
    let pads_hex = fs::read(&file_path)
        .await
        .map_err(|e| DaemonError::IoError(format!("Failed to read file {}: {}", file_path, e)))?;

    let pads_hex: Vec<PadInfo> = serde_json::from_slice(&pads_hex)
        .map_err(|e| DaemonError::IoError(format!("Failed to parse pads hex: {}", e)))?;

    // Call the method which returns StorageStats directly
    let stats = mutant.import_raw_pads_private_key(pads_hex).await;
    tracing::info!("Imported file successfully: {:?}", stats);

    let response = Response::Import(ImportResponse {
        result: ImportResult {
            nb_keys_imported: 0, // TODO
        },
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

async fn handle_export(
    req: ExportRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling Export request");

    let destination_path = req.destination_path.clone();

    let pads_hex = mutant.export_raw_pads_private_key().await?;

    let pads_hex = serde_json::to_vec(&pads_hex)
        .map_err(|e| DaemonError::IoError(format!("Failed to serialize pads hex: {}", e)))?;

    fs::write(&destination_path, pads_hex).await.map_err(|e| {
        DaemonError::IoError(format!("Failed to write file {}: {}", destination_path, e))
    })?;

    let response = Response::Export(ExportResponse {
        result: ExportResult {
            nb_keys_exported: 0, // TODO
        },
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

async fn handle_stop_task(
    req: StopTaskRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = req.task_id;
    tracing::info!(task_id = %task_id, "Received request to stop task");
    let mut tasks_guard = tasks.write().await;

    if let Some(entry) = tasks_guard.get_mut(&task_id) {
        // Check if the task is in a state that can be stopped
        match entry.task.status {
            TaskStatus::Pending | TaskStatus::InProgress => {
                tracing::info!(task_id = %task_id, "Attempting to abort task");
                // Abort the task if the handle exists
                if let Some(handle) = &entry.abort_handle {
                    handle.abort();
                    entry.abort_handle = None; // Remove handle after aborting
                    entry.task.status = TaskStatus::Stopped;
                    entry.task.result =
                        TaskResult::Error("Task stopped by user request".to_string());
                    tracing::info!(task_id = %task_id, "Task aborted and status set to Stopped");

                    // Send TaskStoppedResponse instead of TaskResult
                    let final_update = Response::TaskStopped(TaskStoppedResponse { task_id });
                    // Send needs the original update_tx, not the cloned one from spawn
                    if update_tx.send(final_update).is_err() {
                        tracing::warn!(task_id = %task_id, "Failed to send stop confirmation to client (channel closed)");
                    }
                } else {
                    // This case might happen if the task finished *just* before stop was processed
                    // or if it's a non-abortable task type (though currently all are)
                    tracing::warn!(task_id = %task_id, "Stop requested, but task had no abort handle (might have already finished?). Current status: {:?}", entry.task.status);
                    // If already completed/failed, don't change status back to Stopped
                    if entry.task.status != TaskStatus::Completed
                        && entry.task.status != TaskStatus::Failed
                    {
                        entry.task.status = TaskStatus::Stopped;
                        entry.task.result = TaskResult::Error(
                            "Task stopped by user request (handle missing)".to_string(),
                        );
                    }
                }
            }
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped => {
                // Task is already in a final state, nothing to do
                tracing::info!(task_id = %task_id, "Stop requested, but task is already in final state: {:?}", entry.task.status);
            }
        }
    } else {
        tracing::warn!(task_id = %task_id, "Stop requested for unknown task ID");
        // Optionally send an error response back?
        let error_response = Response::Error(ErrorResponse {
            error: format!("Task not found: {}", task_id),
            original_request: None, // Don't have original request string here easily
        });
        if update_tx.send(error_response).is_err() {
            tracing::warn!(task_id = %task_id, "Failed to send task not found error to client (channel closed)");
        }
    }

    Ok(())
}
