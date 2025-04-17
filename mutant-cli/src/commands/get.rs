use crate::callbacks::StyledProgressBar;
use crate::callbacks::get::create_get_callback;
use futures::TryStreamExt;
use indicatif::MultiProgress;
use log::{debug, error, info};

use mutant_lib::{DataError, Error as LibError, MutAnt};
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn handle_get(
    mutant: MutAnt,
    key: String,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!("CLI: Handling Get command: key={}", key);

    let (download_pb_opt, callback) = create_get_callback(multi_progress, quiet);

    match mutant.fetch_with_progress(&key, Some(callback)).await {
        Ok(value_bytes) => {
            clear_pb(&download_pb_opt);
            if let Err(e) = io::stdout().write_all(&value_bytes) {
                eprintln!("Error writing fetched data to stdout: {}", e);
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            let error_message = match e {
                LibError::Data(DataError::KeyNotFound(k)) => {
                    format!("Error: Key '{}' not found.", k)
                }
                LibError::Data(DataError::InternalError(msg)) if msg.contains("incomplete") => {
                    format!("Cannot fetch key '{}': {}", key, msg)
                }

                _ => format!("Error getting data for key '{}': {}", key, e),
            };

            eprintln!("{}", error_message);
            abandon_pb(&download_pb_opt, error_message);

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
        log::warn!("clear_pb: Could not acquire lock to clear progress bar.");
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
        log::warn!("abandon_pb: Could not acquire lock to abandon progress bar.");
    }
}
