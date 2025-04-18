use crate::callbacks::StyledProgressBar;
use crate::callbacks::put::create_put_callback;
use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::MutAnt;
use mutant_lib::error::{DataError, Error as LibError};
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

    let (res_pb_opt, upload_pb_opt, confirm_pb_opt, callback) =
        create_put_callback(multi_progress, quiet);

    let result = if force {
        debug!(
            "CLI: Force flag is set. Removing existing key '{}' before storing.",
            key
        );
        // First, attempt to remove the key. We ignore RecordNotFound errors.
        match mutant.remove(&key).await {
            Ok(_) => {
                debug!(
                    "Successfully removed existing key '{}' or it didn't exist.",
                    key
                );
            }
            Err(LibError::Data(DataError::KeyNotFound(_))) => {
                debug!(
                    "Key '{}' not found during remove, proceeding with store.",
                    key
                );
                // This is expected if the key didn't exist, continue to store.
            }
            Err(e) => {
                // Any other error during remove is fatal for the update operation
                eprintln!("Error removing key '{}' before update: {}", key, e);
                let msg = format!("Error removing key before update: {}", e);
                abandon_pb(&res_pb_opt, msg.clone());
                abandon_pb(&upload_pb_opt, msg.clone());
                abandon_pb(&confirm_pb_opt, msg);
                return ExitCode::FAILURE;
            }
        }
        // Now, store the new data
        debug!("Storing new data for key '{}' after removal.", key);
        mutant
            .store_with_progress(key.clone(), &data_vec, Some(callback))
            .await
    } else {
        debug!("CLI: Checking status of key '{}' before storing.", key);
        match mutant.get_key_details(&key).await {
            Ok(Some(details)) => {
                if details.is_finished {
                    // Key exists and is complete
                    eprintln!(
                        "Error: Key '{}' already exists and is complete. Use --force to overwrite.",
                        key
                    );
                    // Abandon progress bars if they were initialized (though they likely weren't)
                    let msg = format!("Key '{}' already exists and is complete.", key);
                    abandon_pb(&res_pb_opt, msg.clone());
                    abandon_pb(&upload_pb_opt, msg.clone());
                    abandon_pb(&confirm_pb_opt, msg);
                    return ExitCode::FAILURE;
                } else {
                    // Key exists but is incomplete - resume
                    if !quiet {
                        eprintln!("Resuming incomplete upload for key '{}'...", key);
                    }
                    debug!(
                        "CLI: Key '{}' exists but is incomplete. Resuming upload.",
                        key
                    );
                    mutant
                        .store_with_progress(key.clone(), &data_vec, Some(callback))
                        .await
                }
            }
            Ok(None) => {
                // Key does not exist - normal store
                debug!(
                    "CLI: Key '{}' does not exist. Proceeding with new upload.",
                    key
                );
                mutant
                    .store_with_progress(key.clone(), &data_vec, Some(callback))
                    .await
            }
            Err(e) => {
                // Error checking key details
                eprintln!("Error checking status for key '{}': {}", key, e);
                let msg = format!("Error checking key status: {}", e);
                abandon_pb(&res_pb_opt, msg.clone());
                abandon_pb(&upload_pb_opt, msg.clone());
                abandon_pb(&confirm_pb_opt, msg);
                return ExitCode::FAILURE;
            }
        }
    };

    match result {
        Ok(_) => {
            debug!("Put operation successful for key: {}", key);

            clear_pb(&res_pb_opt);
            clear_pb(&upload_pb_opt);
            clear_pb(&confirm_pb_opt);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let error_message = match e {
                LibError::OperationCancelled => "Operation cancelled.".to_string(),
                _ => format!(
                    "Error during {}: {}",
                    if force { "update" } else { "store" },
                    e,
                ),
            };

            eprintln!("{}", error_message);

            abandon_pb(&res_pb_opt, error_message.clone());
            abandon_pb(&upload_pb_opt, error_message.clone());
            abandon_pb(&confirm_pb_opt, error_message);

            ExitCode::FAILURE
        }
    }
}

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
