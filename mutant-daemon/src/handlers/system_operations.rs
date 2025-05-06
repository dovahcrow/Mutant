use std::sync::Arc;
use uuid::Uuid;

use crate::error::Error as DaemonError;
use crate::TaskMap;
use mutant_lib::MutAnt;
use mutant_protocol::{
    HealthCheckCallback, HealthCheckEvent, HealthCheckRequest, PurgeCallback, PurgeEvent,
    PurgeRequest, Response, SyncCallback, SyncEvent, SyncRequest, Task, TaskCreatedResponse,
    TaskProgress, TaskResult, TaskResultResponse, TaskResultType, TaskStatus, TaskType,
    TaskUpdateResponse,
};

use super::common::UpdateSender;

pub(crate) async fn handle_sync(
    req: SyncRequest,
    update_tx: UpdateSender,
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
    let task_entry = crate::TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

    Ok(())
}

pub(crate) async fn handle_purge(
    req: PurgeRequest,
    update_tx: UpdateSender,
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
    let task_entry = crate::TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

    Ok(())
}

pub(crate) async fn handle_health_check(
    req: HealthCheckRequest,
    update_tx: UpdateSender,
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
    let task_entry = crate::TaskEntry {
        task, // The task struct created earlier
        abort_handle: Some(abort_handle),
    };

    // Insert the TaskEntry into the map *after* spawning
    {
        tasks.write().await.insert(task_id, task_entry);
    }

    Ok(())
}
