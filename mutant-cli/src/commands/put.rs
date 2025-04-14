use crate::callbacks::put::create_put_callback;
use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::error::Error;
use mutant_lib::mutant::MutAnt;
use std::io::{self, Read};
use std::process::ExitCode;

pub async fn handle_put(
    mutant: MutAnt,
    key: String,
    value: Option<String>,
    force: bool,
    multi_progress: &MultiProgress,
) -> ExitCode {
    debug!(
        "CLI: Handling Put command: key={}, value provided: {}, force: {}",
        key,
        value.is_some(),
        force
    );
    let bytes_to_store = match value {
        Some(v) => v.into_bytes(),
        None => {
            let mut buffer = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut buffer) {
                eprintln!("Error reading value from stdin: {}", e);
                return ExitCode::FAILURE;
            }
            buffer
        }
    };

    let put_multi_progress = multi_progress.clone();
    let (reservation_pb, upload_pb, confirm_pb, _confirm_counter_arc, callback) =
        create_put_callback(&put_multi_progress);

    // Spawn the background task for drawing progress bars
    let _progress_jh = tokio::spawn(async move {
        // Just holding multi_progress keeps it drawing
        // If we needed to wait, we'd await the JoinHandle (_progress_jh)
        // outside the spawn block.
        let _ = multi_progress; // Keep the clone alive
        // Add a small delay or loop to prevent immediate exit if needed,
        // although holding it should suffice.
        // tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)).await;
    });

    let result = if force {
        debug!("Forcing update for key: {}", key);
        mutant
            .update_with_progress(&key, &bytes_to_store, Some(callback))
            .await
    } else {
        mutant
            .store_with_progress(
                &key,
                &bytes_to_store,
                Some(callback),
                _confirm_counter_arc.clone(),
            )
            .await
    };

    match result {
        Ok(_) => {
            debug!("handle_put: Result Ok. Attempting multi_progress.clear()...");
            if let Err(e) = multi_progress.clear() {
                warn!("Failed to clear multi-progress bars: {}", e);
            }
            debug!("handle_put: multi_progress.clear() finished. Returning ExitCode::SUCCESS...");
            ExitCode::SUCCESS
        }
        Err(Error::OperationCancelled) => {
            eprintln!("Operation cancelled.");
            let clear_pb = |pb_opt: &std::sync::Arc<
                tokio::sync::Mutex<Option<crate::callbacks::StyledProgressBar>>,
            >| {
                if let Ok(mut guard) = pb_opt.try_lock() {
                    if let Some(pb) = guard.take() {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                        } else {
                            pb.finish_and_clear();
                        }
                    }
                } else {
                    warn!("clear_pb: Could not lock progress bar mutex.");
                }
            };
            clear_pb(&reservation_pb);
            clear_pb(&upload_pb);
            clear_pb(&confirm_pb);
            ExitCode::FAILURE
        }
        Err(Error::KeyAlreadyExists(_)) if !force => {
            eprintln!("Key '{}' already exists. Use --force to overwrite.", key);
            ExitCode::FAILURE
        }
        Err(Error::KeyNotFound(_)) if force => {
            eprintln!(
                "Key not found for forced update: {}. Use put without --force to create.",
                key
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!(
                "Error during {}: {}",
                if force { "update" } else { "store" },
                e
            );
            let fail_msg = "Operation Failed".to_string();
            let abandon_pb = |pb_opt: &std::sync::Arc<
                tokio::sync::Mutex<Option<crate::callbacks::StyledProgressBar>>,
            >| {
                if let Ok(mut guard) = pb_opt.try_lock() {
                    if let Some(pb) = guard.take() {
                        pb.abandon_with_message(fail_msg.clone());
                    }
                } else {
                    warn!("abandon_pb: Could not lock progress bar mutex.");
                }
            };
            abandon_pb(&reservation_pb);
            abandon_pb(&upload_pb);
            abandon_pb(&confirm_pb);
            ExitCode::FAILURE
        }
    }
}
