use crate::callbacks;
use crate::callbacks::progress::ProgressWrapper;
use crate::connect_to_daemon;
use crate::history::append_history_entry;
use crate::history::FetchHistoryEntry;
use crate::utils::{ensure_progress_cleared, format_elapsed_time};
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use mutant_protocol::TaskResult;
use mutant_protocol::TaskResultType;
use std::time::Instant;

pub async fn handle_get(
    key: String,
    destination_path: String,
    public: bool,
    background: bool,
    quiet: bool,
) -> Result<()> {
    if background {
        // For background tasks, we'll just start the operation and return
        // without waiting for it to complete
        let mut client = connect_to_daemon().await?;
        let (_start_task, _progress_rx) =
            client.get(&key, &destination_path, public).await?;

        // Don't await the start_task, just let it run in the background

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    // Start timing the operation
    let start_time = Instant::now();

    let (start_task, progress_rx) = client.get(&key, &destination_path, public).await?;

    // Create the progress bar wrapper
    // Keep it in scope until the end of the function to ensure progress bars are properly cleaned up
    let progress = if !quiet {
        let progress = ProgressWrapper::new();
        callbacks::get::create_get_progress(progress_rx, progress.multi_progress());
        Some(progress)
    } else {
        None
    };

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                if error.contains("Key not found") ||
                   error.contains("No pads found for key") ||
                   error.contains("upload is not finished") {
                    eprintln!("{} Key '{}' not found.", "Error:".bright_red(), key);
                } else {
                    eprintln!("{} {}", "Error:".bright_red(), error);
                }
            }
            TaskResult::Result(result) => match result {
                TaskResultType::Get(result) => {
                    // Calculate and format elapsed time
                    let time_str = format_elapsed_time(start_time.elapsed());

                    // Ensure progress bars are cleared before displaying final message
                    if let Some(p) = &progress {
                        ensure_progress_cleared(p.multi_progress());
                    }

                    println!(
                        "{} Get task completed in {}. Result saved to {} on daemon.",
                        "•".bright_green(),
                        time_str,
                        destination_path
                    );

                    if public {
                        let history_entry = FetchHistoryEntry {
                            address: key,
                            size: result.size,
                            fetched_at: Utc::now(),
                        };
                        append_history_entry(history_entry);
                    }
                }
                _ => {
                    eprintln!("{} {}", "Error:".bright_red(), "Unknown task result");
                }
            },
            TaskResult::Pending => {
                println!("{} Get task pending.", "•".bright_yellow());
            }
        },
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    Ok(())
}
