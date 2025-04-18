use crate::callbacks::StyledProgressBar;
use crate::callbacks::put::create_put_callback;
use indicatif::MultiProgress;
use log::{debug, info, warn};
use mutant_lib::MutAnt;
use mutant_lib::error::{DataError, Error as LibError};
use mutant_lib::storage::ScratchpadAddress;
use std::io::{self, Read};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn handle_put(
    mutant: MutAnt,
    key: String,
    value: Option<String>,
    force: bool,
    public: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!(
        "CLI: Handling Put command: key={}, value_is_some={}, force={}, public={}",
        key,
        value.is_some(),
        force,
        public
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

    let result: Result<Option<ScratchpadAddress>, LibError> = if public {
        debug!(
            "CLI: Storing data publicly for key '{}'. Force={}",
            key, force
        );
        mutant
            .store_public(key.clone(), &data_vec, Some(callback))
            .await
            .map(Some)
    } else {
        if force {
            debug!(
                "CLI: Force flag is set for private store. Removing existing key '{}' before storing.",
                key
            );
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
                }
                Err(e) => {
                    eprintln!("Error removing key '{}' before update: {}", key, e);
                    let msg = format!("Error removing key before update: {}", e);
                    abandon_pb(&res_pb_opt, msg.clone());
                    abandon_pb(&upload_pb_opt, msg.clone());
                    abandon_pb(&confirm_pb_opt, msg);
                    return ExitCode::FAILURE;
                }
            }
            debug!("Storing new private data for key '{}' after removal.", key);
            mutant
                .store_with_progress(key.clone(), &data_vec, Some(callback))
                .await
                .map(|_| None)
        } else {
            debug!(
                "CLI: Checking status of private key '{}' before storing.",
                key
            );
            match mutant.get_key_details(&key).await {
                Ok(Some(details)) => {
                    if details.is_finished {
                        eprintln!(
                            "Error: Private key '{}' already exists and is complete. Use --force to overwrite.",
                            key
                        );
                        let msg = format!("Private key '{}' already exists and is complete.", key);
                        abandon_pb(&res_pb_opt, msg.clone());
                        abandon_pb(&upload_pb_opt, msg.clone());
                        abandon_pb(&confirm_pb_opt, msg);
                        return ExitCode::FAILURE;
                    } else {
                        if !quiet {
                            eprintln!("Resuming incomplete private upload for key '{}'...", key);
                        }
                        debug!(
                            "CLI: Private key '{}' exists but is incomplete. Resuming upload.",
                            key
                        );
                        mutant
                            .store_with_progress(key.clone(), &data_vec, Some(callback))
                            .await
                            .map(|_| None)
                    }
                }
                Ok(None) => {
                    debug!(
                        "CLI: Private key '{}' does not exist. Proceeding with new upload.",
                        key
                    );
                    mutant
                        .store_with_progress(key.clone(), &data_vec, Some(callback))
                        .await
                        .map(|_| None)
                }
                Err(e) => {
                    eprintln!("Error checking status for private key '{}': {}", key, e);
                    let msg = format!("Error checking private key status: {}", e);
                    abandon_pb(&res_pb_opt, msg.clone());
                    abandon_pb(&upload_pb_opt, msg.clone());
                    abandon_pb(&confirm_pb_opt, msg);
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    match result {
        Ok(public_address_opt) => {
            debug!(
                "Put operation successful for key: {} (public={})",
                key, public
            );

            clear_pb(&res_pb_opt);
            clear_pb(&upload_pb_opt);
            clear_pb(&confirm_pb_opt);

            if let Some(address) = public_address_opt {
                if !quiet {
                    println!("{}", address);
                } else {
                    info!("Public address for key '{}': {}", key, address);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let context = if public {
                "public store"
            } else {
                if force {
                    "private update"
                } else {
                    "private store"
                }
            };
            let error_message = match e {
                LibError::OperationCancelled => "Operation cancelled.".to_string(),
                _ => format!("Error during {}: {}", context, e,),
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
