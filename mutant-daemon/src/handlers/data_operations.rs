use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::Error as DaemonError;
use super::{TaskEntry, TaskMap, ActiveKeysMap, try_register_key, release_key, is_public_only_mode, PUBLIC_ONLY_ERROR_MSG};
use mutant_lib::storage::ScratchpadAddress;
use mutant_lib::MutAnt;
use mutant_protocol::{
    ErrorResponse, GetCallback, GetEvent, GetRequest, GetResult, MultipartChunkRequest,
    MultipartChunkResponse, MultipartCompleteRequest, MultipartInitRequest, MultipartInitResponse,
    PutCallback, PutEvent, PutRequest, PutResult, PutSource, Response, RmRequest, RmSuccessResponse,
    Task, TaskCreatedResponse, TaskProgress, TaskResult, TaskResultResponse, TaskResultType,
    TaskStatus, TaskType, TaskUpdateResponse
};

use super::common::UpdateSender;

pub(crate) async fn handle_put(
    req: PutRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
    active_keys: ActiveKeysMap,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    // Check if we're in public-only mode
    if is_public_only_mode() {
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: PUBLIC_ONLY_ERROR_MSG.to_string(),
                original_request: Some(original_request_str.to_string()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();

    // Get source info for logging
    let (source_info, data_arc) = match &req.source {
        PutSource::FilePath(path) => {
            let source_path = path.clone();

            // Try to register the key for this task
            try_register_key(
                &active_keys,
                &user_key,
                task_id,
                TaskType::Put,
                &update_tx,
                original_request_str,
            ).await?;

            // Read data into an Arc<Vec<u8>> to avoid cloning the whole data later.
            let data_bytes_vec = fs::read(path).await.map_err(|e| {
                // Release the key if we fail to read the file
                let active_keys_clone = active_keys.clone();
                let user_key_clone = user_key.clone();
                tokio::spawn(async move {
                    release_key(&active_keys_clone, &user_key_clone).await;
                });

                DaemonError::IoError(format!(
                    "Failed to read source file {}: {}",
                    path, e
                ))
            })?;

            (format!("file:{}", source_path), Arc::new(data_bytes_vec))
        },
        PutSource::Bytes(bytes) => {
            // Try to register the key for this task
            try_register_key(
                &active_keys,
                &user_key,
                task_id,
                TaskType::Put,
                &update_tx,
                original_request_str,
            ).await?;

            let filename = req.filename.clone().unwrap_or_else(|| "direct-bytes".to_string());
            (format!("bytes:{}", filename), Arc::new(bytes.clone()))
        },
        PutSource::MultipartRef(total_size) => {
            // Try to register the key for this task
            try_register_key(
                &active_keys,
                &user_key,
                task_id,
                TaskType::Put,
                &update_tx,
                original_request_str,
            ).await?;

            let filename = req.filename.clone().unwrap_or_else(|| "multipart-upload".to_string());
            // For MultipartRef, we create an empty Vec with the specified capacity
            // The actual data will be filled in by the multipart chunks
            (format!("multipart:{}", filename), Arc::new(Vec::with_capacity(*total_size)))
        }
    };

    let task = Task {
        id: task_id,
        task_type: TaskType::Put,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
        key: Some(user_key.clone()),
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
    let active_keys_clone = active_keys.clone();
    let user_key_clone = user_key.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;
        let data_to_put = data_arc_clone; // Use the cloned Arc
        let active_keys = active_keys_clone;
        let user_key = user_key_clone;

        log::info!("Starting PUT task: task_id={}, user_key={}, source={}", task_id, user_key, source_info);

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
                            // For public keys, get the index pad address from the master index
                            let public_address = if req.public {
                                // Get the index pad address from the master index
                                match mutant.get_public_index_address(&user_key).await {
                                    Ok(addr) => Some(addr),
                                    Err(e) => {
                                        log::warn!("Failed to get public index address for key {}: {}", user_key, e);
                                        None
                                    }
                                }
                            } else {
                                None
                            };

                            entry.task.status = TaskStatus::Completed;
                            entry.task.result = TaskResult::Result(TaskResultType::Put(PutResult {
                                public_address,
                            }));
                            entry.abort_handle = None; // Task finished, remove handle
                            log::info!("PUT task completed successfully: task_id={}, user_key={}, source={}", task_id, user_key, source_info);
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
                            log::error!("PUT task failed: task_id={}, user_key={}, source={}, error={}", task_id, user_key, source_info, e);
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

        // Release the key when the operation completes
        release_key(&active_keys, &user_key).await;
        log::debug!("Released key '{}' after PUT operation", user_key);
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
    active_keys: ActiveKeysMap,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();
    let destination_path = req.destination_path.clone(); // Keep path for logging and writing

    // Try to register the key for this task
    try_register_key(
        &active_keys,
        &user_key,
        task_id,
        TaskType::Get,
        &update_tx,
        original_request_str,
    ).await?;

    let task = Task {
        id: task_id,
        task_type: TaskType::Get,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
        key: Some(user_key.clone()),
    };

    update_tx
        .send(Response::TaskCreated(TaskCreatedResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    // Clone Arc handles *before* moving them into the async block
    let tasks_clone = tasks.clone();
    let mutant_clone = mutant.clone();
    let update_tx_clone_for_spawn = update_tx.clone();
    let active_keys_clone = active_keys.clone();
    let user_key_clone = user_key.clone();

    let task_handle = tokio::spawn(async move {
        // Use the cloned handles inside the spawned task
        let tasks = tasks_clone;
        let mutant = mutant_clone;
        let update_tx = update_tx_clone_for_spawn;
        let active_keys = active_keys_clone;
        let user_key = user_key_clone;

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

        // Release the key when the operation completes
        release_key(&active_keys, &user_key).await;
        log::debug!("Released key '{}' after GET operation", user_key);
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

// Structure to hold multipart upload data
struct MultipartUploadData {
    user_key: String,
    total_size: usize,
    chunks: HashMap<usize, Vec<u8>>,
    received_chunks: usize,
    total_chunks: usize,
    mode: mutant_protocol::StorageMode,
    public: bool,
    no_verify: bool,
    filename: Option<String>,
}

// Global storage for multipart uploads
type MultipartUploads = Arc<RwLock<HashMap<Uuid, Arc<RwLock<MultipartUploadData>>>>>;

lazy_static::lazy_static! {
    static ref MULTIPART_UPLOADS: MultipartUploads = Arc::new(RwLock::new(HashMap::new()));
}

pub(crate) async fn handle_multipart_init(
    req: MultipartInitRequest,
    update_tx: UpdateSender,
    tasks: TaskMap,
    active_keys: ActiveKeysMap,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    // Check if we're in public-only mode
    if is_public_only_mode() {
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: PUBLIC_ONLY_ERROR_MSG.to_string(),
                original_request: Some(original_request_str.to_string()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();

    // Try to register the key for this task
    try_register_key(
        &active_keys,
        &user_key,
        task_id,
        TaskType::Put,
        &update_tx,
        original_request_str,
    ).await?;

    // Calculate total chunks based on a reasonable chunk size (e.g., 1MB)
    let chunk_size = 1024 * 1024; // 1MB chunks
    let total_chunks = (req.total_size + chunk_size - 1) / chunk_size;

    // Create multipart upload data
    let upload_data = MultipartUploadData {
        user_key: user_key.clone(),
        total_size: req.total_size,
        chunks: HashMap::new(),
        received_chunks: 0,
        total_chunks,
        mode: req.mode,
        public: req.public,
        no_verify: req.no_verify,
        filename: req.filename,
    };

    // Store the upload data
    MULTIPART_UPLOADS.write().await.insert(task_id, Arc::new(RwLock::new(upload_data)));

    // Create a task entry
    let task = Task {
        id: task_id,
        task_type: TaskType::Put,
        status: TaskStatus::Pending,
        progress: None,
        result: TaskResult::Pending,
        key: Some(user_key.clone()),
    };

    // Insert the task into the task map
    let task_entry = TaskEntry {
        task,
        abort_handle: None, // No abort handle yet
    };
    tasks.write().await.insert(task_id, task_entry);

    // Send the response
    update_tx
        .send(Response::MultipartInit(MultipartInitResponse { task_id }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    log::info!("Initialized multipart upload: task_id={}, user_key={}, total_size={}, total_chunks={}",
               task_id, user_key, req.total_size, total_chunks);

    Ok(())
}

pub(crate) async fn handle_multipart_chunk(
    req: MultipartChunkRequest,
    update_tx: UpdateSender,
) -> Result<(), DaemonError> {
    let task_id = req.task_id;
    let chunk_index = req.chunk_index;
    let chunk_data = req.data;

    // Get the upload data
    let uploads = MULTIPART_UPLOADS.read().await;
    let upload_data_arc = uploads.get(&task_id).ok_or_else(|| {
        DaemonError::Internal(format!("Multipart upload not found for task_id={}", task_id))
    })?;

    // Update the upload data
    {
        let mut upload_data = upload_data_arc.write().await;
        upload_data.chunks.insert(chunk_index, chunk_data.clone());
        upload_data.received_chunks += 1;

        log::debug!("Received chunk {} for task_id={}, size={} bytes, progress={}/{}",
                   chunk_index, task_id, chunk_data.len(), upload_data.received_chunks, upload_data.total_chunks);
    }

    // Send the response
    update_tx
        .send(Response::MultipartChunk(MultipartChunkResponse {
            chunk_index,
            bytes_received: chunk_data.len(),
        }))
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

pub(crate) async fn handle_multipart_complete(
    req: MultipartCompleteRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    tasks: TaskMap,
    active_keys: ActiveKeysMap,
) -> Result<(), DaemonError> {
    let task_id = req.task_id;

    // Get the upload data
    let mut uploads = MULTIPART_UPLOADS.write().await;
    let upload_data_arc = uploads.remove(&task_id).ok_or_else(|| {
        DaemonError::Internal(format!("Multipart upload not found for task_id={}", task_id))
    })?;

    // Assemble the complete data
    let upload_data = upload_data_arc.read().await;
    let mut complete_data = Vec::with_capacity(upload_data.total_size);

    // Sort chunks by index and append them
    let mut sorted_chunks: Vec<(&usize, &Vec<u8>)> = upload_data.chunks.iter().collect();
    sorted_chunks.sort_by_key(|(idx, _)| *idx);

    for (_, chunk) in sorted_chunks {
        complete_data.extend_from_slice(chunk);
    }

    // Check if we received all the data
    if complete_data.len() != upload_data.total_size {
        return Err(DaemonError::Internal(format!(
            "Incomplete multipart upload: received {} bytes, expected {} bytes",
            complete_data.len(), upload_data.total_size
        )));
    }

    // Create a PutRequest with the assembled data
    let put_request = PutRequest {
        user_key: upload_data.user_key.clone(),
        source: PutSource::Bytes(complete_data),
        filename: upload_data.filename.clone(),
        mode: upload_data.mode.clone(),
        public: upload_data.public,
        no_verify: upload_data.no_verify,
    };

    // Update task status
    {
        let mut tasks_guard = tasks.write().await;
        if let Some(entry) = tasks_guard.get_mut(&task_id) {
            entry.task.status = TaskStatus::InProgress;

            // Send task update
            update_tx
                .send(Response::TaskUpdate(TaskUpdateResponse {
                    task_id,
                    status: TaskStatus::InProgress,
                    progress: Some(TaskProgress::Put(PutEvent::Starting {
                        total_chunks: 0, // Will be set by the actual put operation
                        initial_written_count: 0,
                        initial_confirmed_count: 0,
                        chunks_to_reserve: 0,
                    })),
                }))
                .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;
        }
    }

    log::info!("Completed multipart upload assembly: task_id={}, user_key={}, total_size={} bytes",
               task_id, upload_data.user_key, upload_data.total_size);

    // Now handle the put operation with the assembled data
    // We'll reuse the existing task_id
    drop(upload_data); // Release the lock before calling handle_put

    // Call handle_put with the assembled data
    handle_put(put_request, update_tx, mutant, tasks, active_keys, "multipart_complete").await
}

pub(crate) async fn handle_rm(
    req: RmRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
    active_keys: ActiveKeysMap,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    // Check if we're in public-only mode
    if is_public_only_mode() {
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: PUBLIC_ONLY_ERROR_MSG.to_string(),
                original_request: Some(original_request_str.to_string()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    let task_id = Uuid::new_v4();
    let user_key = req.user_key.clone();
    log::info!("Starting RM task: user_key={}", user_key);

    // Check if the key exists first
    let key_exists = mutant.contains_key(&user_key).await;

    if !key_exists {
        log::info!("RM task for non-existent key: user_key={}", user_key);
        // Return an error for non-existent keys
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: format!("Key '{}' not found", user_key),
                original_request: Some(original_request_str.to_string()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    // Try to register the key for this task
    try_register_key(
        &active_keys,
        &user_key,
        task_id,
        TaskType::Rm,
        &update_tx,
        original_request_str,
    ).await?;

    let result = mutant.rm(&user_key).await;

    // Release the key after the operation completes
    release_key(&active_keys, &user_key).await;

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
