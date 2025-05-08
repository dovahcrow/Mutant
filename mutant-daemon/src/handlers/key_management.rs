use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use mutant_protocol::{ErrorResponse, Response, TaskType};
use crate::error::Error as DaemonError;
use super::common::UpdateSender;

/// Tracks which keys are currently being operated on and by which task
#[derive(Debug, Clone)]
pub struct ActiveKeyEntry {
    pub task_id: uuid::Uuid,
    pub task_type: TaskType,
}

/// Map of active keys to their task information
pub type ActiveKeysMap = Arc<RwLock<HashMap<String, ActiveKeyEntry>>>;

/// Attempts to register a key for a specific task
/// Returns Ok(()) if the key was successfully registered
/// Returns Err with an error response if the key is already in use
pub async fn try_register_key(
    active_keys: &ActiveKeysMap,
    key: &str,
    task_id: uuid::Uuid,
    task_type: TaskType,
    update_tx: &UpdateSender,
    original_request_str: &str,
) -> Result<(), DaemonError> {
    let mut keys_guard = active_keys.write().await;

    if let Some(entry) = keys_guard.get(key) {
        // Key is already in use
        let error_msg = format!(
            "Key '{}' is already being used by another operation (task ID: {}, type: {:?}). Only one operation per key is allowed at a time.",
            key, entry.task_id, entry.task_type
        );

        log::warn!("{}", error_msg);

        // Send error response to client
        update_tx
            .send(Response::Error(ErrorResponse {
                error: error_msg,
                original_request: Some(original_request_str.to_string()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

        // Return early with error
        return Err(DaemonError::Internal(format!("Key '{}' is already in use", key)));
    }

    // Key is not in use, register it
    let task_type_clone = task_type.clone(); // Clone for logging
    keys_guard.insert(
        key.to_string(),
        ActiveKeyEntry {
            task_id,
            task_type,
        },
    );

    log::debug!("Registered key '{}' for task ID: {}, type: {:?}", key, task_id, task_type_clone);

    Ok(())
}

/// Releases a key after an operation is complete
pub async fn release_key(active_keys: &ActiveKeysMap, key: &str) {
    let mut keys_guard = active_keys.write().await;

    if keys_guard.remove(key).is_some() {
        log::debug!("Released key '{}'", key);
    } else {
        log::warn!("Attempted to release key '{}' that wasn't registered", key);
    }
}
