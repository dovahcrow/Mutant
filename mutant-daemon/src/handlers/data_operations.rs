use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;

use crate::error::Error as DaemonError;
use super::{TaskEntry, TaskMap};
use mutant_lib::storage::ScratchpadAddress;
use mutant_lib::MutAnt;
use mutant_protocol::{
    ErrorResponse, GetCallback, GetEvent, GetRequest, GetResult, PutCallback, PutEvent, PutRequest,
    Response, RmRequest, RmSuccessResponse, Task, TaskCreatedResponse, TaskProgress, TaskResult,
    TaskResultResponse, TaskResultType, TaskStatus, TaskType, TaskUpdateResponse,
};

use super::common::UpdateSender;

pub(crate) async fn handle_put(
    req: PutRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();
    let source_path = req.source_path.clone(); // Keep path for logging

    // Read data into an Arc<Vec<u8>> to avoid cloning the whole data later.
    let data_bytes_vec = fs::read(&req.source_path).await.map_err(|e| {
        DaemonError::IoError(format!(
            "Failed to read source file {}: {}",
            req.source_path, e
        ))
    })?;
    let data_arc = Arc::new(data_bytes_vec); // Wrap in Arc

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
    let data_arc_clone = data_arc.clone(); // Clone the Arc for the task

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;
        let data_to_put = data_arc_clone; // Use the cloned Arc

        log::info!("Starting PUT task: task_id={}, user_key={}, source_path={}", task_id, user_key, source_path);

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
                        log::warn!("Received PUT progress update for task not InProgress (status: {:?}). Ignoring. task_id={}", entry.task.status, task_id);
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
                data_to_put, // Pass the Arc<Vec<u8>>
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
                            log::info!("PUT task completed successfully: task_id={}, user_key={}, source_path={}", task_id, user_key, source_path);
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
                            log::error!("PUT task failed: task_id={}, user_key={}, source_path={}, error={}", task_id, user_key, source_path, e);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    log::info!("PUT task was stopped before completion: task_id={}", task_id);
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                log::warn!("Task entry removed before PUT completion? task_id={}", task_id);
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

pub(crate) async fn handle_get(
    req: GetRequest,
    update_tx: UpdateSender,
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

        log::info!("Starting GET task: task_id={}, user_key={}, destination_path={}", task_id, user_key, destination_path);

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
                        log::warn!("Received GET progress update for task not InProgress (status: {:?}). Ignoring. task_id={}", entry.task.status, task_id);
                        return Ok(false); // Indicate to stop sending updates if task is no longer InProgress
                    }
                }
                drop(tasks_guard);
                Ok(true)
            })
        });

        // Check if the key exists first for private keys
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
            // Check if the key exists first for better error messages
            if !mutant.contains_key(&user_key).await {
                Err(mutant_lib::error::Error::Internal(format!(
                    "Key '{}' not found",
                    user_key
                )))
            } else {
                mutant.get(&user_key, Some(callback)).await // Pass callback
            }
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
                            log::info!("GET task completed successfully: task_id={}, user_key={}, destination_path={}, bytes_written={}", task_id, user_key, destination_path, data_bytes.len());
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
                            log::error!("GET task failed: task_id={}, user_key={}, destination_path={}, error={}", task_id, user_key, destination_path, error_msg);
                            Some(Response::TaskResult(TaskResultResponse {
                                task_id,
                                status: TaskStatus::Failed,
                                result: entry.task.result.clone(),
                            }))
                        }
                    }
                } else {
                    log::info!("GET task was stopped before completion: task_id={}", task_id);
                    entry.abort_handle = None; // Ensure handle is cleared if stopped
                    None // No final result to send if stopped
                }
            } else {
                log::warn!("Task entry removed before GET completion? task_id={}", task_id);
                None
            }
        };
        if let Some(response) = final_response {
            if update_tx.send(response).is_err() {
                log::debug!("Client disconnected before final GET result sent: task_id={}", task_id);
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

pub(crate) async fn handle_rm(
    req: RmRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    let user_key = req.user_key.clone();
    log::info!("Starting RM task: user_key={}", user_key);

    // Check if the key exists first
    let key_exists = mutant.contains_key(&user_key).await;

    if !key_exists {
        log::info!("RM task for non-existent key: user_key={}", user_key);
        // Return success even for non-existent keys to make the CLI behavior consistent
        return update_tx
            .send(Response::RmSuccess(RmSuccessResponse { user_key }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    let result = mutant.rm(&user_key).await;

    let response = match result {
        Ok(_) => {
            log::info!("RM task completed successfully: user_key={}", user_key);
            Response::RmSuccess(RmSuccessResponse { user_key })
        }
        Err(e) => {
            log::error!("RM task failed: user_key={}, error={}", user_key, e);
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
