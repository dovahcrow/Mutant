use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures_util::{
    sink::SinkExt,
    stream::{SplitSink, StreamExt},
};
use mutant_lib::{error::Error as LibError, storage::StorageMode, MutAnt};
use tokio::sync::mpsc;
use uuid::Uuid;
use warp::ws::{Message, WebSocket};

use crate::{error::Error as DaemonError, TaskMap};
use mutant_protocol::{
    ErrorResponse, GetRequest, ListTasksRequest, PutCallback, PutEvent, PutRequest,
    QueryTaskRequest, Request, Response, Task, TaskCreatedResponse, TaskListEntry,
    TaskListResponse, TaskProgress, TaskResult, TaskResultResponse, TaskStatus, TaskType,
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
    update_tx: mpsc::UnboundedSender<Response>,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    match request {
        Request::Put(put_req) => handle_put(put_req, update_tx, mutant, tasks).await?,
        Request::Get(get_req) => handle_get(get_req, update_tx, mutant, tasks).await?,
        Request::QueryTask(query_req) => handle_query_task(query_req, update_tx, tasks).await?,
        Request::ListTasks(list_req) => handle_list_tasks(list_req, update_tx, tasks).await?,
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

    let data_bytes = BASE64_STANDARD
        .decode(&req.data_b64)
        .map_err(DaemonError::Base64Decode)?;

    let task = Task {
        id: task_id,
        task_type: TaskType::Put,
        status: TaskStatus::Pending,
        progress: None,
        result: None,
    };
    {
        tasks.write().await.insert(task_id, task.clone());
    }

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tokio::spawn(async move {
        tracing::info!(task_id = %task_id, user_key = %user_key, "Starting PUT task");

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

        let result = mutant
            .put(&user_key, &data_bytes, StorageMode::Medium, false)
            .await;

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                match result {
                    Ok(_addr) => {
                        task.status = TaskStatus::Completed;
                        task.result = Some(TaskResult {
                            data: None,
                            error: None,
                        });
                        tracing::info!(task_id = %task_id, user_key = %user_key, "PUT task completed successfully");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Completed,
                            result: task.result.clone(),
                        }))
                    }
                    Err(e) => {
                        task.status = TaskStatus::Failed;
                        task.result = Some(TaskResult {
                            data: None,
                            error: Some(e.to_string()),
                        });
                        tracing::error!(task_id = %task_id, user_key = %user_key, error = %e, "PUT task failed");
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

    let task = Task {
        id: task_id,
        task_type: TaskType::Get,
        status: TaskStatus::Pending,
        progress: None,
        result: None,
    };
    {
        tasks.write().await.insert(task_id, task.clone());
    }

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    tokio::spawn(async move {
        tracing::info!(task_id = %task_id, user_key = %user_key, "Starting GET task");
        let start_update_res = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                task.status = TaskStatus::InProgress;
                let progress = TaskProgress::Legacy {
                    message: "Starting GET operation".to_string(),
                };
                task.progress = Some(progress.clone());
                Some(Response::TaskUpdate(TaskUpdateResponse {
                    task_id,
                    status: TaskStatus::InProgress,
                    progress: Some(progress),
                }))
            } else {
                None
            }
        };
        if let Some(update) = start_update_res {
            if update_tx.send(update).is_err() {
                tracing::warn!(task_id = %task_id, "Client disconnected before GET task started");
                return; // Exit if client disconnected
            }
        }

        let result = mutant.get(&user_key).await;

        let final_response = {
            let mut tasks_guard = tasks.write().await;
            if let Some(task) = tasks_guard.get_mut(&task_id) {
                match result {
                    Ok(data_bytes) => {
                        let data_b64 = BASE64_STANDARD.encode(&data_bytes);
                        task.status = TaskStatus::Completed;
                        task.result = Some(TaskResult {
                            data: Some(data_b64),
                            error: None,
                        });
                        tracing::info!(task_id = %task_id, user_key = %user_key, "GET task completed successfully");
                        Some(Response::TaskResult(TaskResultResponse {
                            task_id,
                            status: TaskStatus::Completed,
                            result: task.result.clone(),
                        }))
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        task.status = TaskStatus::Failed;
                        task.result = Some(TaskResult {
                            data: None,
                            error: Some(error_msg.clone()),
                        });
                        tracing::error!(task_id = %task_id, user_key = %user_key, "GET task failed: {}", error_msg);
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

async fn handle_query_task(
    req: QueryTaskRequest,
    update_tx: mpsc::UnboundedSender<Response>,
    tasks: TaskMap,
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
            original_request: None,
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
