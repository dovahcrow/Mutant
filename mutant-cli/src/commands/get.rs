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
    destination_path: Option<String>,
    public: bool,
    background: bool,
    stdout: bool,
    quiet: bool,
) -> Result<()> {
    // Determine if we're streaming data (no destination path or stdout flag)
    let stream_data = stdout || destination_path.is_none();

    // If background mode is requested, make sure we have a destination path
    if background && stream_data {
        eprintln!("{} Background mode requires a destination path.", "Error:".bright_red());
        return Ok(());
    }

    if background {
        // For background tasks, we'll just start the operation and return
        // without waiting for it to complete
        let mut client = connect_to_daemon().await?;
        let dest = destination_path.as_ref().unwrap();
        let (_start_task, _progress_rx, _) =
            client.get(&key, Some(dest), public, false).await?;

        // Don't await the start_task, just let it run in the background
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    // Start timing the operation
    let start_time = Instant::now();

    // Call get with the appropriate parameters based on streaming mode
    let (start_task, progress_rx, data_stream_rx) = if stream_data {
        // Streaming mode - no destination path
        client.get(&key, None, public, true).await?
    } else {
        // Normal mode - with destination path
        client.get(&key, destination_path.as_ref(), public, false).await?
    };

    // Create the progress bar wrapper
    // Keep it in scope until the end of the function to ensure progress bars are properly cleaned up
    let progress = if !quiet && !stream_data {
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

                    // Handle streaming data if requested
                    if stream_data {
                        if let Some(mut data_rx) = data_stream_rx {
                            // Collect all data chunks
                            let mut all_data = Vec::new();

                            while let Some(data_result) = data_rx.recv().await {
                                match data_result {
                                    Ok(chunk) => {
                                        all_data.extend(chunk);
                                    }
                                    Err(e) => {
                                        eprintln!("{} Error receiving data chunk: {}", "Error:".bright_red(), e);
                                        break;
                                    }
                                }
                            }

                            // Output the data
                            if stdout {
                                // Write directly to stdout
                                use std::io::{self, Write};
                                io::stdout().write_all(&all_data)?;
                            } else {
                                // Write to a file in the current directory
                                let filename = key.split('/').last().unwrap_or(&key);
                                std::fs::write(filename, all_data)?;

                                println!(
                                    "{} Get task completed in {}. Result saved to {}",
                                    "•".bright_green(),
                                    time_str,
                                    filename
                                );
                            }
                        } else {
                            eprintln!("{} No data stream received", "Error:".bright_red());
                        }
                    } else {
                        // Ensure progress bars are cleared before displaying final message
                        if let Some(p) = &progress {
                            ensure_progress_cleared(p.multi_progress());
                        }

                        println!(
                            "{} Get task completed in {}. Result saved to {} on daemon.",
                            "•".bright_green(),
                            time_str,
                            destination_path.unwrap_or_default()
                        );
                    }

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
