use crate::callbacks::get::create_get_callback;
use indicatif::MultiProgress;
use log::debug;
use mutant_lib::Error;
use mutant_lib::mutant::MutAnt;
use std::io::{self, Write};
use std::process::ExitCode;

pub async fn handle_get(mutant: MutAnt, key: String, multi_progress: &MultiProgress) -> ExitCode {
    debug!("CLI: Handling Get command: key={}", key);

    let get_multi_progress = multi_progress.clone();
    let (download_pb_opt, callback) = create_get_callback(&get_multi_progress);

    match mutant.fetch_with_progress(&key, Some(callback)).await {
        Ok(value_bytes) => {
            if let Some(pb) = download_pb_opt.lock().unwrap().take() {
                if !pb.is_finished() {
                    pb.finish_and_clear();
                    debug!("handle_get: Success - Finishing and clearing progress bar.");
                }
            }

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

            if let Some(pb) = download_pb_opt.lock().unwrap().take() {
                if !pb.is_finished() {
                    let abandon_message = match e {
                        Error::UploadIncomplete(_) => "Upload incomplete".to_string(),
                        _ => error_message.clone(),
                    };
                    pb.abandon_with_message(abandon_message);
                    debug!("handle_get: Error - Abandoning progress bar ({:?}).", e);
                } else {
                    debug!("handle_get: Error - Progress bar already finished.");
                }
            }
            ExitCode::FAILURE
        }
    }
}
