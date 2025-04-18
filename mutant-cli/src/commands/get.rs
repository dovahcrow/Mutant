use crate::callbacks::StyledProgressBar;
use crate::callbacks::get::create_get_callback;
use indicatif::MultiProgress;
use log::debug;
use mutant_lib::MutAnt;
use mutant_lib::error::{DataError, Error as LibError};
use mutant_lib::storage::ScratchpadAddress;
use std::io::{self, Write};
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn handle_get(
    mutant: MutAnt,
    key_or_address: String,
    public: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!(
        "CLI: Handling Get command: key_or_address={}, public={}",
        key_or_address, public
    );

    let (download_pb_opt, callback) = create_get_callback(multi_progress, quiet);

    let result: Result<Vec<u8>, LibError> = if public {
        debug!(
            "CLI: Fetching public data using address '{}'",
            key_or_address
        );
        match ScratchpadAddress::from_hex(&key_or_address) {
            Ok(address) => mutant
                .fetch_public(address, Some(callback))
                .await
                .map(|bytes| bytes.to_vec()),
            Err(e) => {
                eprintln!(
                    "Error: Invalid public address format '{}': {}",
                    key_or_address, e
                );
                abandon_pb(
                    &download_pb_opt,
                    format!("Invalid public address format: {}", e),
                );
                return ExitCode::FAILURE;
            }
        }
    } else {
        debug!("CLI: Fetching private data using key '{}'", key_or_address);
        mutant
            .fetch_with_progress(&key_or_address, Some(callback))
            .await
    };

    match result {
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
            let identifier = if public { "address" } else { "key" };
            let error_message = match e {
                LibError::Data(DataError::KeyNotFound(k)) => {
                    format!("Error: {} '{}' not found.", identifier, k)
                }
                LibError::Data(DataError::InternalError(msg)) if msg.contains("incomplete") => {
                    format!("Cannot fetch {}: {}", key_or_address, msg)
                }
                _ => format!(
                    "Error getting data for {} '{}': {}",
                    identifier, key_or_address, e
                ),
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
