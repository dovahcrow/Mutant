use log::{debug, error, trace, warn};
use mutant_protocol::{
    ErrorResponse, ExportResponse, ImportResponse, ListKeysResponse, Response,
    RmSuccessResponse, Task, TaskCreatedResponse, TaskListResponse, TaskProgress,
    TaskResult, TaskResultResponse, TaskStatus, TaskStoppedResponse, TaskType,
    TaskUpdateResponse,
};

use crate::{
    error::ClientError, ClientTaskMap, PendingRequestKey, PendingRequestMap, PendingSender,
    TaskChannelsMap,
};

use super::MutantClient;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

impl MutantClient {
    /// Processes a deserialized response from the server
    pub fn process_response(
        response: Response,
        tasks: &ClientTaskMap,
        task_channels: &TaskChannelsMap,
        pending_requests: &PendingRequestMap,
    ) {
        log::warn!("PROCESS RESPONSE: {:#?}", response);
        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::TaskCreation);

                if let Some(PendingSender::TaskCreation(sender, channels, task_type)) =
                    pending_sender
                {
                    tasks.lock().unwrap().insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type,
                            status: TaskStatus::Pending,
                            progress: None,
                            result: TaskResult::Pending,
                            key: None,
                        },
                    );

                    task_channels.lock().unwrap().insert(task_id, channels);

                    if sender.send(Ok(task_id)).is_err() {
                        warn!("Failed to send TaskCreated response to waiting future (receiver dropped?)");
                        tasks.lock().unwrap().remove(&task_id);
                        task_channels.lock().unwrap().remove(&task_id);
                    }
                } else {
                    warn!(
                        "Received TaskCreated ({}) but no matching request was pending.",
                        task_id
                    );
                }
            }
            Response::TaskUpdate(TaskUpdateResponse {
                task_id,
                status,
                progress,
            }) => {
                let mut tasks_guard = tasks.lock().unwrap();
                let task_exists = tasks_guard.contains_key(&task_id);

                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.progress = progress.clone();

                    if let Some(progress_update) = progress {
                        if let Some((_, progress_tx)) = task_channels.lock().unwrap().get(&task_id)
                        {
                            if progress_tx.send(Ok(progress_update)).is_err() {
                                warn!("Failed to send progress update for task {}", task_id);
                            }
                        }
                    }
                } else {
                    warn!(
                        "Received TaskUpdate for unknown task {}, creating entry.",
                        task_id
                    );
                    let task_type = match &progress {
                        Some(TaskProgress::Put(_)) => TaskType::Put,
                        _ => TaskType::Get,
                    };
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type,
                            status,
                            progress,
                            result: TaskResult::Pending,
                            key: None,
                        },
                    );
                }

                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::QueryTask);
                if let Some(PendingSender::QueryTask(sender)) = pending_sender {
                    if let Some(task) = tasks_guard.get(&task_id) {
                        if sender.send(Ok(task.clone())).is_err() {
                            warn!("Failed to send TaskUpdate response to QueryTask request (receiver dropped)");
                        }
                    } else {
                        warn!("Task {} not found when trying to respond to QueryTask request after TaskUpdate.", task_id);
                        let _ = sender.send(Err(ClientError::TaskNotFound(task_id)));
                    }
                } else if task_exists {
                    // No pending query, just updated the task
                } else {
                    // No pending query and task was created by this update - already logged warning
                }
            }
            Response::TaskResult(TaskResultResponse {
                task_id,
                status,
                result,
            }) => {
                let mut tasks_guard = tasks.lock().unwrap();
                let _task_existed = tasks_guard.contains_key(&task_id);

                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.result = result.clone();

                    if let Some((completion_tx, _)) = task_channels.lock().unwrap().remove(&task_id)
                    {
                        if completion_tx.send(Ok(result.clone())).is_err() {
                            warn!(
                                "Failed to send final task result for task {} (receiver dropped)",
                                task_id
                            );
                        }
                    }
                } else {
                    warn!(
                        "Received TaskResult for unknown task {}, creating entry.",
                        task_id
                    );
                    // FIXME: Nonsense
                    let task_type = if let TaskResult::Error(_) = result {
                        TaskType::Get
                    } else {
                        TaskType::Put
                    };
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type,
                            status,
                            result,
                            progress: None,
                            key: None,
                        },
                    );
                }

                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::QueryTask);
                if let Some(PendingSender::QueryTask(sender)) = pending_sender {
                    if let Some(task) = tasks_guard.get(&task_id) {
                        if sender.send(Ok(task.clone())).is_err() {
                            warn!("Failed to send TaskResult response to QueryTask request (receiver dropped)");
                        }
                    } else {
                        warn!("Task {} not found when trying to respond to QueryTask request after TaskResult.", task_id);
                        let _ = sender.send(Err(ClientError::TaskNotFound(task_id)));
                    }
                }
            }
            Response::TaskList(TaskListResponse { tasks: task_list }) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::ListTasks);
                if let Some(PendingSender::ListTasks(sender)) = pending_sender {
                    if sender.send(Ok(task_list)).is_err() {
                        warn!("Failed to send TaskList response (receiver dropped)");
                    }
                } else {
                    warn!("Received TaskList but no ListTasks request was pending");
                }
            }
            Response::Error(ErrorResponse {
                error,
                original_request: _,
            }) => {
                error!(
                    "Server error received: {}. Check server logs for details.",
                    error
                );

                let mut requests = pending_requests.lock().unwrap();

                if let Some(PendingSender::TaskCreation(sender, _, _)) =
                    requests.remove(&PendingRequestKey::TaskCreation)
                {
                    error!("Error occurred during task creation: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(PendingSender::ListTasks(sender)) =
                    requests.remove(&PendingRequestKey::ListTasks)
                {
                    error!("Error occurred during task list request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(PendingSender::QueryTask(sender)) =
                    requests.remove(&PendingRequestKey::QueryTask)
                {
                    error!("Error occurred during task query request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(PendingSender::Rm(sender)) =
                    requests.remove(&PendingRequestKey::Rm)
                {
                    error!("Error occurred during rm request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(PendingSender::ListKeys(sender)) =
                    requests.remove(&PendingRequestKey::ListKeys)
                {
                    error!("Error occurred during list keys request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(PendingSender::Stats(sender)) =
                    requests.remove(&PendingRequestKey::Stats)
                {
                    error!("Error occurred during stats request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else if let Some(PendingSender::Import(sender)) =
                    requests.remove(&PendingRequestKey::Import)
                {
                    error!("Error occurred during import request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else if let Some(PendingSender::Export(sender)) =
                    requests.remove(&PendingRequestKey::Export)
                {
                    error!("Error occurred during export request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error)));
                } else {
                    warn!("Received server error, but no matching pending request found.");
                }
            }
            Response::RmSuccess(RmSuccessResponse { user_key: _ }) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::Rm);
                if let Some(PendingSender::Rm(sender)) = pending_sender {
                    if sender.send(Ok(())).is_err() {
                        warn!("Failed to send RM success response (receiver dropped)");
                    }
                } else {
                    warn!("Received RM success response but no Rm request was pending");
                }
            }
            Response::ListKeys(ListKeysResponse { keys }) => {
                log::debug!("Received ListKeys response with {} keys", keys.len());

                // Safely get the pending sender
                let pending_sender = match pending_requests.lock() {
                    Ok(mut guard) => guard.remove(&PendingRequestKey::ListKeys),
                    Err(e) => {
                        error!("Failed to lock pending_requests mutex: {:?}", e);
                        None
                    }
                };

                // Process the sender if it exists
                if let Some(PendingSender::ListKeys(sender)) = pending_sender {
                    debug!("Sending ListKeys response to waiting future");

                    // Clone the keys to avoid any potential memory issues
                    let keys_clone = keys.clone();

                    // Send the response and handle errors
                    match sender.send(Ok(keys_clone)) {
                        Ok(_) => debug!("Successfully sent ListKeys response"),
                        Err(_) => warn!("Failed to send ListKeys response (receiver dropped)")
                    }
                } else {
                    warn!("Received ListKeys response but no ListKeys request was pending");
                }
            }
            Response::Stats(stats_response) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::Stats);
                if let Some(PendingSender::Stats(sender)) = pending_sender {
                    if sender.send(Ok(stats_response)).is_err() {
                        warn!("Failed to send Stats response (receiver dropped)");
                    }
                } else {
                    warn!("Received Stats response but no Stats request was pending");
                }
            }
            Response::Import(ImportResponse { result }) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::Import);
                if let Some(PendingSender::Import(sender)) = pending_sender {
                    if sender.send(Ok(result)).is_err() {
                        warn!("Failed to send Import response (receiver dropped)");
                    }
                } else {
                    warn!("Received Import response but no Import request was pending");
                }
            }
            Response::Export(ExportResponse { result }) => {
                let pending_sender = pending_requests
                    .lock()
                    .unwrap()
                    .remove(&PendingRequestKey::Export);
                if let Some(PendingSender::Export(sender)) = pending_sender {
                    if sender.send(Ok(result)).is_err() {
                        warn!("Failed to send Export response (receiver dropped)");
                    }
                } else {
                    warn!("Received Export response but no Export request was pending");
                }
            }
            Response::TaskStopped(res) => handle_task_stopped(res, pending_requests.clone()),
        }
    }

    pub async fn next_response(&mut self) -> Option<Result<Response, ClientError>> {
        if let Some(receiver) = &mut self.receiver {
            // Use a loop to handle non-text messages without recursion
            loop {
                // Use the async next() method from nash-ws to wait for the next message
                match receiver.next().await {
                    Some(Ok(message)) => {
                        // Process the message
                        match message {
                            nash_ws::Message::Text(text) => {
                                match serde_json::from_str::<Response>(&text) {
                                    Ok(response) => {
                                        Self::process_response(
                                            response.clone(),
                                            &self.tasks,
                                            &self.task_channels,
                                            &self.pending_requests,
                                        );
                                        return Some(Ok(response));
                                    }
                                    Err(e) => {
                                        error!("Failed to deserialize response: {}", e);
                                        return Some(Err(ClientError::DeserializationError(e)));
                                    }
                                }
                            }
                            nash_ws::Message::Binary(_) => {
                                debug!("Received binary WebSocket message, expected text");
                                // Continue the loop to wait for the next message
                                continue;
                            }
                            _ => {
                                debug!("Received unexpected WebSocket message type");
                                // Continue the loop to wait for the next message
                                continue;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {:?}", e);
                        return Some(Err(ClientError::WebSocketError(format!("{:?}", e))));
                    }
                    None => {
                        debug!("WebSocket connection closed");
                        return None;
                    }
                }
            }
        } else {
            None
        }
    }
}

// Correctly define the function to accept Arc<Mutex<...>>
fn handle_task_stopped(
    res: TaskStoppedResponse,
    pending_requests_mutex: Arc<Mutex<HashMap<PendingRequestKey, PendingSender>>>,
) {
    trace!("Received TaskStopped response for task {}", res.task_id);
    // Lock the mutex to get mutable access to the map
    let mut pending_requests = match pending_requests_mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            error!(
                "Mutex poisoned when handling TaskStopped for task {}: {}. Recovering.",
                res.task_id, poisoned
            );
            poisoned.into_inner() // Recover the data even if poisoned
        }
    };

    // Find the corresponding sender using the correct key type
    if let Some(PendingSender::StopTask(sender)) =
        pending_requests.remove(&PendingRequestKey::StopTask)
    {
        // Sender expects Result<TaskStoppedResponse, ClientError>
        if sender.send(Ok(res.clone())).is_err() {
            warn!(
                "Failed to send TaskStopped result for task {}: receiver dropped",
                res.task_id
            );
        }
    } else {
        warn!(
            "Received TaskStopped response for unknown/mismatched request key (task_id: {})",
            res.task_id
        );
    }
    // Lock guard is dropped here, releasing the mutex
}
