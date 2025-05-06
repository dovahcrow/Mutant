use crate::error::Error as DaemonError;
use crate::TaskMap;
use mutant_protocol::{
    ErrorResponse, ListTasksRequest, QueryTaskRequest, Response, StopTaskRequest, TaskListEntry,
    TaskListResponse, TaskResultResponse, TaskStatus, TaskStoppedResponse, TaskUpdateResponse,
};

use super::common::UpdateSender;

pub(crate) async fn handle_query_task(
    req: QueryTaskRequest,
    update_tx: UpdateSender,
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

pub(crate) async fn handle_list_tasks(
    _req: ListTasksRequest,
    update_tx: UpdateSender,
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

pub(crate) async fn handle_stop_task(
    req: StopTaskRequest,
    update_tx: UpdateSender,
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
                        mutant_protocol::TaskResult::Error("Task stopped by user request".to_string());
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
                        entry.task.result = mutant_protocol::TaskResult::Error(
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
