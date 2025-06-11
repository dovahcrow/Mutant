use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error as DaemonError;
use mutant_protocol::{
    ErrorResponse, FileSystemEntry, GetFileInfoRequest, GetFileInfoResponse, ListDirectoryRequest,
    ListDirectoryResponse, Response,
};

use super::common::UpdateSender;

/// Get the user's home directory
fn get_home_directory() -> Result<PathBuf, DaemonError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| DaemonError::Internal("Unable to determine user home directory".to_string()))
}

/// Convert a web path (starting with "/") to an actual filesystem path within the home directory
/// For security, all paths are restricted to the user's home directory
fn resolve_secure_path(web_path: &str) -> Result<PathBuf, DaemonError> {
    let home_dir = get_home_directory()?;

    // Normalize the web path - remove leading "/" and resolve ".." components
    let normalized_path = web_path.trim_start_matches('/');

    // Build the actual path within the home directory
    let mut actual_path = home_dir.clone();

    if !normalized_path.is_empty() {
        // Split path components and validate each one
        for component in normalized_path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if component == ".." {
                // Don't allow going above home directory
                if actual_path != home_dir {
                    actual_path.pop();
                }
            } else {
                actual_path.push(component);
            }
        }
    }

    // Ensure the resolved path is still within the home directory
    if !actual_path.starts_with(&home_dir) {
        return Err(DaemonError::Internal("Access denied: path outside home directory".to_string()));
    }

    Ok(actual_path)
}

/// Convert an actual filesystem path back to a web path relative to home directory
fn to_web_path(actual_path: &Path) -> Result<String, DaemonError> {
    let home_dir = get_home_directory()?;

    if let Ok(relative_path) = actual_path.strip_prefix(&home_dir) {
        if relative_path.as_os_str().is_empty() {
            Ok("/".to_string())
        } else {
            Ok(format!("/{}", relative_path.to_string_lossy()))
        }
    } else {
        Err(DaemonError::Internal("Path is outside home directory".to_string()))
    }
}

pub(crate) async fn handle_list_directory(
    req: ListDirectoryRequest,
    update_tx: UpdateSender,
) -> Result<(), DaemonError> {
    log::debug!("Handling ListDirectory request for web path: {}", req.path);

    // Convert web path to secure filesystem path
    let actual_path = match resolve_secure_path(&req.path) {
        Ok(path) => path,
        Err(e) => {
            let error_msg = format!("Invalid path: {}", e);
            log::warn!("{}", error_msg);
            return update_tx
                .send(Response::Error(ErrorResponse {
                    error: error_msg,
                    original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
                }))
                .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
        }
    };

    log::debug!("Resolved to actual path: {}", actual_path.display());

    // Check if the path exists and is a directory
    if !actual_path.exists() {
        let error_msg = format!("Path does not exist: {}", req.path);
        log::warn!("{}", error_msg);
        return update_tx
            .send(Response::Error(ErrorResponse {
                error: error_msg,
                original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
            }))
            .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
    }

    if !actual_path.is_dir() {
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
    let entries = match fs::read_dir(&actual_path) {
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

                // Convert actual filesystem path back to web path
                let web_path = match to_web_path(&entry_path) {
                    Ok(path) => path,
                    Err(e) => {
                        log::warn!("Failed to convert path to web path: {}", e);
                        continue; // Skip this entry
                    }
                };

                filesystem_entries.push(FileSystemEntry {
                    name,
                    path: web_path,
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

    let home_directory = get_home_directory()?.to_string_lossy().to_string();

    let response = Response::ListDirectory(ListDirectoryResponse {
        path: req.path,
        entries: filesystem_entries,
        home_directory,
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))
}

pub(crate) async fn handle_get_file_info(
    req: GetFileInfoRequest,
    update_tx: UpdateSender,
) -> Result<(), DaemonError> {
    log::debug!("Handling GetFileInfo request for web path: {}", req.path);

    // Convert web path to secure filesystem path
    let actual_path = match resolve_secure_path(&req.path) {
        Ok(path) => path,
        Err(e) => {
            let error_msg = format!("Invalid path: {}", e);
            log::warn!("{}", error_msg);
            return update_tx
                .send(Response::Error(ErrorResponse {
                    error: error_msg,
                    original_request: Some(serde_json::to_string(&req).unwrap_or_default()),
                }))
                .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)));
        }
    };

    log::debug!("Resolved to actual path: {}", actual_path.display());

    let exists = actual_path.exists();
    let is_directory = actual_path.is_dir();
    let readable = actual_path.exists() && fs::metadata(&actual_path).is_ok();

    let (size, modified) = if exists {
        match fs::metadata(&actual_path) {
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
