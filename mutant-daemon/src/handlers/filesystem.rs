use std::fs;
use std::path::Path;

use crate::error::Error as DaemonError;
use mutant_protocol::{
    ErrorResponse, FileSystemEntry, GetFileInfoRequest, GetFileInfoResponse, ListDirectoryRequest,
    ListDirectoryResponse, Response,
};

use super::common::UpdateSender;

pub(crate) async fn handle_list_directory(
    req: ListDirectoryRequest,
    update_tx: UpdateSender,
) -> Result<(), DaemonError> {
    log::debug!("Handling ListDirectory request for path: {}", req.path);

    let path = Path::new(&req.path);

    // Check if the path exists and is a directory
    if !path.exists() {
        let error_msg = format!("Path does not exist: {}", req.path);
        log::warn!("{}", error_msg);
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: error_msg,
                original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    if !path.is_dir() {
        let error_msg = format!("Path is not a directory: {}", req.path);
        log::warn!("{}", error_msg);
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: error_msg,
                original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    // Read directory contents
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) => {
            let error_msg = format!("Failed to read directory {}: {}", req.path, e);
            log::error!("{}", error_msg);
            return update_tx
                .send(Response::Error(ErrorResponse {
                    error: error_msg,
                    original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
                }))
                .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
        }
    };

    let mut filesystem_entries = Vec::new();

    for entry in entries {
        match entry {
            Ok(dir_entry) => {
                let entry_path = dir_entry.path();
                let name = entry_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Unknown".to_string());

                let is_directory = entry_path.is_dir();
                let size = if is_directory {
                    None
                } else {
                    match fs::metadata(&entry_path) {
                        Ok(metadata) => Some(metadata.len()),
                        Err(_) => None,
                    }
                };

                let modified = match fs::metadata(&entry_path) {
                    Ok(metadata) => match metadata.modified() {
                        Ok(time) => time
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .map(|d| d.as_secs()),
                        Err(_) => None,
                    },
                    Err(_) => None,
                };

                filesystem_entries.push(FileSystemEntry {
                    name,
                    path: entry_path.to_string_lossy().to_string(),
                    is_directory,
                    size,
                    modified,
                });
            }
            Err(e) => {
                log::warn!("Failed to read directory entry: {}", e);
                // Continue with other entries
            }
        }
    }

    // Sort entries: directories first, then files, both alphabetically
    filesystem_entries.sort_by(|a, b| {
        match (a.is_directory, b.is_directory) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    let response = Response::ListDirectory(ListDirectoryResponse {
        path: req.path,
        entries: filesystem_entries,
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))
}

pub(crate) async fn handle_get_file_info(
    req: GetFileInfoRequest,
    update_tx: UpdateSender,
) -> Result<(), DaemonError> {
    log::debug!("Handling GetFileInfo request for path: {}", req.path);

    let path = Path::new(&req.path);

    let exists = path.exists();
    let is_directory = path.is_dir();
    let readable = path.exists() && fs::metadata(path).is_ok();

    let (size, modified) = if exists {
        match fs::metadata(path) {
            Ok(metadata) => {
                let size = if is_directory { None } else { Some(metadata.len()) };
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                (size, modified)
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    let response = Response::GetFileInfo(GetFileInfoResponse {
        path: req.path,
        exists,
        is_directory,
        size,
        modified,
        readable,
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))
}
