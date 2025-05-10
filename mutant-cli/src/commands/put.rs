use crate::callbacks;
use crate::callbacks::progress::ProgressWrapper;
use crate::connect_to_daemon;
use crate::utils::{ensure_progress_cleared, format_elapsed_time};
use anyhow::Result;
use colored::Colorize;
use mutant_protocol::{StorageMode, TaskResult};
use std::path::PathBuf;
use std::time::Instant;

pub async fn handle_put(
    key: String,
    file: String,
    public: bool,
    mode: StorageMode,
    no_verify: bool,
    background: bool,
    quiet: bool,
) -> Result<()> {
    // Convert to absolute path to ensure daemon can find the file
    let path_buf = PathBuf::from(&file);
    let absolute_path = if path_buf.is_absolute() {
        path_buf
    } else {
        std::env::current_dir()?.join(path_buf)
    };
    let source_path = absolute_path.to_string_lossy().to_string();

    if background {
        // For background tasks, we'll just start the operation and return
        // without waiting for it to complete
        let mut client = connect_to_daemon().await?;
        let (_start_task, _progress_rx) = client
            .put(&key, &source_path, mode, public, no_verify)
            .await?;

        // Don't await the start_task, just let it run in the background

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    // Start timing the operation
    let start_time = Instant::now();

    let (start_task, progress_rx) = client
        .put(&key, &source_path, mode, public, no_verify)
        .await?;

    // Create the progress bar wrapper
    // Keep it in scope until the end of the function to ensure progress bars are properly cleaned up
    let progress = if !quiet {
        let progress = ProgressWrapper::new();
        callbacks::put::create_put_progress(progress_rx, progress.multi_progress().clone());
        Some(progress)
    } else {
        None
    };

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                eprintln!("{} {}", "Error:".bright_red(), error);
            }
            TaskResult::Result(result) => {
                // Calculate and format elapsed time
                let time_str = format_elapsed_time(start_time.elapsed());

                // Ensure progress bars are cleared before displaying final message
                if let Some(p) = &progress {
                    ensure_progress_cleared(p.multi_progress());
                }

                println!("{} Upload complete! (took {})", "•".bright_green(), time_str);

                // If this is a public key, display the index address
                if public {
                    if let mutant_protocol::TaskResultType::Put(put_result) = result {
                        if let Some(public_address) = put_result.public_address {
                            println!("{} Public index address: {}", "•".bright_blue(), public_address);
                        }
                    }
                }
            }
            TaskResult::Pending => {
                println!("{} Upload pending.", "•".bright_yellow());
            }
        },
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    // // Keep the MultiProgress instance alive until the task is complete
    // drop(multi_progress);

    Ok(())
}
