use crate::callbacks::StyledProgressBar;
use crate::callbacks::put::create_put_callback;
use indicatif::MultiProgress;
use log::{debug, warn};
// Use new top-level re-exports
use mutant_lib::{Error as LibError, MutAnt, data::DataError};
use std::io::{self, Read};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn handle_put(
    mutant: MutAnt,
    key: String,
    value: Option<String>,
    force: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!(
        "CLI: Handling Put command: key={}, value_is_some={}, force={}",
        key,
        value.is_some(),
        force
    );

    // Get data as Vec<u8>
    let data_vec: Vec<u8> = match value {
        Some(v) => {
            debug!("handle_put: Using value from argument");
            v.into_bytes()
        }
        None => {
            if !quiet && atty::is(atty::Stream::Stdin) {
                eprintln!("Reading value from stdin... (Ctrl+D to end)");
            }
            debug!("handle_put: Reading value from stdin");
            let mut buffer = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut buffer) {
                eprintln!("Error reading value from stdin: {}", e);
                return ExitCode::FAILURE;
            }
            buffer
        }
    };

    // Conditionally create callbacks based on quiet flag
    // Adjusted destructuring for two bars
    let (create_pb_opt, confirm_pb_opt, callback) = create_put_callback(multi_progress, quiet);

    // Pass data as slice &[u8]
    let result = if force {
        debug!("Forcing update for key: {}", key);
        mutant
            .update_with_progress(key.clone(), &data_vec, Some(callback))
            .await
    } else {
        mutant
            .store_with_progress(key.clone(), &data_vec, Some(callback))
            .await
    };

    match result {
        Ok(_) => {
            debug!("Put operation successful for key: {}", key);
            // Clear only the two remaining bars
            clear_pb(&create_pb_opt);
            clear_pb(&confirm_pb_opt);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let error_message = match e {
                LibError::Data(DataError::KeyNotFound(k)) if force => {
                    format!(
                        "Cannot force update non-existent key '{}'. Use put without --force.",
                        k
                    )
                }
                LibError::Data(DataError::KeyAlreadyExists(k)) if !force => {
                    format!("Key '{}' already exists. Use --force to overwrite.", k)
                }
                LibError::OperationCancelled => "Operation cancelled.".to_string(),
                _ => format!(
                    "Error during {}: {}",
                    if force { "update" } else { "store" },
                    e,
                ),
            };

            eprintln!("{}", error_message);
            // Abandon only the two remaining bars
            abandon_pb(&create_pb_opt, error_message.clone());
            abandon_pb(&confirm_pb_opt, error_message);

            ExitCode::FAILURE
        }
    }
}

// Helper functions to clear or abandon progress bars - kept local to put.rs
fn clear_pb(pb_opt: &Arc<Mutex<Option<StyledProgressBar>>>) {
    if let Ok(mut guard) = pb_opt.try_lock() {
        if let Some(pb) = guard.take() {
            if !pb.is_finished() {
                pb.finish_and_clear();
            }
        }
    } else {
        warn!("clear_pb: Could not acquire lock to clear progress bar.");
    }
}

fn abandon_pb(pb_opt: &Arc<Mutex<Option<StyledProgressBar>>>, message: String) {
    if let Ok(mut guard) = pb_opt.try_lock() {
        if let Some(pb) = guard.take() {
            if !pb.is_finished() {
                pb.abandon_with_message(message);
            }
        }
    } else {
        warn!("abandon_pb: Could not acquire lock to abandon progress bar.");
    }
}
