use log::{debug, error, info, warn};
use mutant_protocol::{
    ErrorResponse, ListTasksRequest, QueryTaskRequest, Request, Response, Task,
    TaskCreatedResponse, TaskId, TaskListEntry, TaskListResponse, TaskProgress, TaskResult,
    TaskResultResponse, TaskStatus, TaskType, TaskUpdateResponse,
};

use crate::{
    error::ClientError, ClientTaskMap, PendingTaskCreationSender, PendingTaskListSender,
    TaskChannelsMap,
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
    ) {
        match response {
            Response::TaskCreated(TaskCreatedResponse { task_id }) => {
                tasks.lock().unwrap().insert(
                    task_id,
                    Task {
                        id: task_id,
                        task_type: TaskType::Get,
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
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get,
                            status,
                            progress,
                            result: None,
                        },
                    );
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
                        let _ = completion_tx.send(Ok(result.unwrap_or_else(|| TaskResult {
                            data: None,
                            error: None,
                        })));
                    }
                } else {
                    tasks_guard.insert(
                        task_id,
                        Task {
                            id: task_id,
                            task_type: TaskType::Get,
                            status,
                            result,
                            progress: None,
                        },
                    );
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
                        }
                    },
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
        } else {
            None
        }
    }
}
