use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

const HISTORY_FILE_NAME: &str = "fetch_history.cbor";

/// Represents an entry for a previously fetched public item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchHistoryEntry {
    /// The address of the index scratchpad for the fetched item.
    pub address: String,
    /// The total size of the fetched data in bytes.
    pub size: usize,
    /// Timestamp indicating when the item was last fetched.
    pub fetched_at: DateTime<Utc>,
}

/// Gets the platform-specific path to the history file.
fn get_history_file_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "MutAnt", "MutAnt")
        .map(|proj_dirs| proj_dirs.config_dir().join(HISTORY_FILE_NAME))
}

/// Loads the fetch history from the designated file.
/// Returns an empty Vec if the file doesn't exist or fails to load/deserialize.
pub fn load_history() -> Vec<FetchHistoryEntry> {
    match get_history_file_path() {
        Some(path) => {
            if !path.exists() {
                debug!(
                    "History file not found at {:?}. Returning empty history.",
                    path
                );
                return Vec::new();
            }
            match File::open(&path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    match serde_json::from_reader(reader) {
                        Ok(history) => {
                            debug!("Successfully loaded history from {:?}", path);
                            history
                        }
                        Err(e) => {
                            warn!(
                                "Failed to deserialize history file {:?}: {}. Returning empty history.",
                                path, e
                            );
                            Vec::new()
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to open history file {:?}: {}. Returning empty history.",
                        path, e
                    );
                    Vec::new()
                }
            }
        }
        None => {
            error!(
                "Could not determine project directory for history file. Returning empty history."
            );
            Vec::new()
        }
    }
}

/// Saves the given fetch history list to the designated file.
/// Overwrites the existing file.
pub fn save_history(history: &Vec<FetchHistoryEntry>) -> Result<(), std::io::Error> {
    match get_history_file_path() {
        Some(path) => {
            debug!("Saving history to {:?}", path);
            // Ensure the config directory exists
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }

            // Open file for writing, create if it doesn't exist, truncate if it does
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;

            let writer = BufWriter::new(file);
            serde_json::to_writer(writer, history).map_err(|e| {
                error!("Failed to serialize history to {:?}: {}", path, e);
                // Convert serde_cbor::Error to std::io::Error for consistent return type
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })?;
            debug!("Successfully saved history to {:?}", path);
            Ok(())
        }
        None => {
            error!("Could not determine project directory for history file. Save failed.");
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine project directory for history file.",
            ))
        }
    }
}

/// Appends a single entry to the fetch history file.
/// Loads the existing history, adds the new entry, and saves it back.
pub fn append_history_entry(new_entry: FetchHistoryEntry) {
    let mut history = load_history();

    // Optional: Check for duplicates based on address and update timestamp, or just append
    if let Some(existing) = history.iter_mut().find(|e| e.address == new_entry.address) {
        debug!("Updating existing history entry for {}", new_entry.address);
        existing.size = new_entry.size;
        existing.fetched_at = new_entry.fetched_at;
    } else {
        debug!("Appending new history entry for {}", new_entry.address);
        history.push(new_entry);
    }

    // Optional: Limit history size?
    // const MAX_HISTORY_SIZE: usize = 100;
    // if history.len() > MAX_HISTORY_SIZE {
    //     history.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at)); // Ensure newest are kept
    //     history.truncate(MAX_HISTORY_SIZE);
    // }

    if let Err(e) = save_history(&history) {
        warn!("Failed to save updated fetch history: {}", e);
    }
}
