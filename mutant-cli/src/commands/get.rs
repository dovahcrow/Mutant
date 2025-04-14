use crate::callbacks::StyledProgressBar;
use crate::callbacks::get::create_get_callback;
use indicatif::MultiProgress;
use log::debug;
use mutant_lib::Error;
use mutant_lib::mutant::MutAnt;
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
                Error::UploadIncomplete(ref key) => {
                    format!(
                        "Cannot fetch key '{}': The upload is not yet complete.",
                        key
                    )
                }
                _ => format!("Error getting data: {}", e),
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
