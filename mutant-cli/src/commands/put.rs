use crate::callbacks::StyledProgressBar;
use crate::callbacks::put::create_put_callback;
use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::MutAnt;
use mutant_lib::error::Error as LibError;
use mutant_lib::storage::ScratchpadAddress;
use std::io::{self, Read};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cli::StorageModeCli;

pub async fn handle_put(
    mut mutant: MutAnt,
    key: String,
    value: Option<String>,
    force: bool,
    public: bool,
    mode: StorageModeCli,
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

    mutant.set_put_callback(callback).await;

    debug!("handle_put: Calling mutant.put(&key, &data_vec).await...");
    let result: Result<ScratchpadAddress, LibError> = if public {
        mutant.put(&key, &data_vec, mode.into(), true).await
    } else {
        mutant.put(&key, &data_vec, mode.into(), false).await
    };
    debug!("handle_put: mutant.put returned: {:?}", result);

    // Explicitly drop mutant to see if drop causes the hang
    debug!("handle_put: Explicitly dropping mutant instance...");
    drop(mutant);
    debug!("handle_put: Mutant instance dropped.");

    // Handle the result (Ok or Err)
    debug!("handle_put: Matching result...");
    match result {
        Ok(address) => {
            debug!("handle_put: Result is Ok. Returning SUCCESS.");
            if public {
                println!("{}", address);
            }
            clear_pb(&res_pb_opt);
            clear_pb(&upload_pb_opt);
            clear_pb(&confirm_pb_opt);
            ExitCode::SUCCESS
        }
        Err(e) => {
            debug!("handle_put: Result is Err({:?}). Returning FAILURE.", e);
            eprintln!("Error: {:?}", e);
            abandon_pb(&res_pb_opt, e.to_string());
            abandon_pb(&upload_pb_opt, e.to_string());
            abandon_pb(&confirm_pb_opt, e.to_string());
            clear_pb(&res_pb_opt);
            clear_pb(&upload_pb_opt);
            clear_pb(&confirm_pb_opt);
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
