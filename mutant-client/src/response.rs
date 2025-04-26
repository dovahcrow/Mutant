use log::{debug, error, warn};
use mutant_protocol::{
    ErrorResponse, Response, RmSuccessResponse, Task, TaskCreatedResponse, TaskListResponse,
    TaskProgress, TaskResult, TaskResultResponse, TaskStatus, TaskType, TaskUpdateResponse,
};

use crate::{
    error::ClientError, ClientTaskMap, PendingRmSender, PendingTaskCreationSender,
    PendingTaskListSender, PendingTaskQuerySender, TaskChannelsMap,
};

use super::MutantClient;

impl MutantClient {
    /// Processes a deserialized response from the server
    pub fn process_response(
        response: Response,
        tasks: &ClientTaskMap,
        task_channels: &TaskChannelsMap,
        pending_task_creation: &PendingTaskCreationSender,
        pending_task_list: &PendingTaskListSender,
        pending_task_query: &PendingTaskQuerySender,
        pending_rm: &PendingRmSender,
    ) {
        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                let task_type =
                    if let Some(pending) = pending_task_creation.lock().unwrap().as_ref() {
                        if pending.channels.is_some() {
                            TaskType::Put
                        } else {
                            TaskType::Get
                        }
                    } else {
                        TaskType::Get
                    };

                tasks.lock().unwrap().insert(
                    task_id,
                    Task {
                        id: task_id,
                        task_type,
                        status: TaskStatus::Pending,
                        progress: None,
                        result: None,
                    },
                );

                if let Some(pending) = pending_task_creation.lock().unwrap().take() {
                    if let Some((completion_tx, progress_tx)) = pending.channels {
                        task_channels
                            .lock()
                            .unwrap()
                            .insert(task_id, (completion_tx, progress_tx));
                    }
                    if pending.sender.send(Ok(task_id)).is_err() {
                        warn!("Failed to send TaskCreated response to waiting future (receiver dropped?)");
                    }
                } else {
                    warn!(
                        "Received TaskCreated ({}) but no request was pending.",
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
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.progress = progress.clone();

                    if let Some(progress) = progress {
                        if let Some((_, progress_tx)) = task_channels.lock().unwrap().get(&task_id)
                        {
                            let _ = progress_tx.send(Ok(progress));
                        }
                    }
                } else {
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
                            result: None,
                        },
                    );
                }

                if let Some(sender) = pending_task_query.lock().unwrap().take() {
                    let task = tasks_guard.get(&task_id).unwrap().clone();
                    if sender.send(Ok(task)).is_err() {
                        warn!("Failed to send TaskUpdate response (receiver dropped)");
                    }
                }
            }
            Response::TaskResult(TaskResultResponse {
                task_id,
                status,
                result,
            }) => {
                let mut tasks_guard = tasks.lock().unwrap();
                if let Some(task) = tasks_guard.get_mut(&task_id) {
                    task.status = status;
                    task.result = result.clone();

                    if let Some((completion_tx, _)) = task_channels.lock().unwrap().remove(&task_id)
                    {
                        let _ = completion_tx
                            .send(Ok(result.unwrap_or_else(|| TaskResult { error: None })));
                    }
                } else {
                    let task_type = if result.as_ref().map_or(false, |r| r.error.is_some()) {
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
                        },
                    );
                }

                if let Some(sender) = pending_task_query.lock().unwrap().take() {
                    let task = tasks_guard.get(&task_id).unwrap().clone();
                    if sender.send(Ok(task)).is_err() {
                        warn!("Failed to send TaskResult response (receiver dropped)");
                    }
                }
            }
            Response::TaskList(TaskListResponse { tasks: task_list }) => {
                if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    if sender.send(Ok(task_list)).is_err() {
                        warn!("Failed to send TaskList response (receiver dropped)");
                    }
                } else {
                    warn!("Received TaskList but no request was pending");
                }
            }
            Response::Error(ErrorResponse {
                error,
                original_request,
            }) => {
                error!(
                    "Server error received: {}. Original request: {:?}",
                    error, original_request
                );
                if let Some(sender) = pending_task_creation.lock().unwrap().take() {
                    error!("Error occurred during task creation: {}", error);
                    let _ = sender
                        .sender
                        .send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(sender) = pending_task_list.lock().unwrap().take() {
                    error!("Error occurred during task list request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                } else if let Some(sender) = pending_task_query.lock().unwrap().take() {
                    error!("Error occurred during task query request: {}", error);
                    let _ = sender.send(Err(ClientError::ServerError(error.clone())));
                }
            }
            Response::RmSuccess(RmSuccessResponse { user_key }) => {
                if let Some(sender) = pending_rm.lock().unwrap().take() {
                    if sender.send(Ok(())).is_err() {
                        warn!("Failed to send RM success response (receiver dropped)");
                    }
                } else {
                    warn!("Received RM success response but no request was pending");
                }
            }
        }
    }

    pub async fn next_response(&mut self) -> Option<Result<Response, ClientError>> {
        if let Some(receiver) = &mut self.receiver {
            loop {
                match receiver.try_recv() {
                    Some(event) => match event {
                        ewebsock::WsEvent::Message(msg) => {
                            if let ewebsock::WsMessage::Text(text) = msg {
                                match serde_json::from_str::<Response>(&text) {
                                    Ok(response) => {
                                        Self::process_response(
                                            response.clone(),
                                            &self.tasks,
                                            &self.task_channels,
                                            &self.pending_task_creation,
                                            &self.pending_task_list,
                                            &self.pending_task_query,
                                            &self.pending_rm,
                                        );
                                        return Some(Ok(response));
                                    }
                                    Err(e) => {
                                        error!("Failed to deserialize response: {}", e);
                                        return Some(Err(ClientError::DeserializationError(e)));
                                    }
                                }
                            } else {
                                debug!("Received non-text WebSocket message");
                            }
                        }
                        ewebsock::WsEvent::Error(e) => {
                            error!("WebSocket error: {}", e);
                            return Some(Err(ClientError::WebSocketError(e.to_string())));
                        }
                        ewebsock::WsEvent::Closed => {
                            debug!("WebSocket connection closed");
                            return None;
                        }
                        ewebsock::WsEvent::Opened => {
                            debug!("WebSocket connection opened");
                            continue;
                        }
                    },
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        continue;
                    }
                }
            }
        } else {
            None
        }
    }
}
