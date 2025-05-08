use crate::callbacks;
use crate::connect_to_daemon;
use crate::history::append_history_entry;
use crate::history::FetchHistoryEntry;
use crate::terminal::ProgressWithDisabledStdin;
use crate::utils::format_elapsed_time;
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
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) =
                client.get(&key, &destination_path, public).await.unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    // Start timing the operation
    let start_time = Instant::now();

    let (start_task, progress_rx) = client.get(&key, &destination_path, public).await?;

    // Create the progress bar wrapper that will disable stdin
    // Keep it in scope until the end of the function to ensure stdin remains disabled
    // and progress bars are properly cleaned up
    let _progress = if !quiet {
        let progress = ProgressWithDisabledStdin::new();
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
