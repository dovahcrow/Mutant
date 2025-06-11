use crate::callbacks;
use crate::callbacks::progress::ProgressWrapper;
use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;
use mutant_protocol::TaskResult;
use mutant_protocol::TaskResultType;

pub async fn handle_health_check(
    key_name: String,
    background: bool,
    recycle: bool,
    quiet: bool,
) -> Result<()> {
    if background {
        // For background tasks, we'll just start the operation and return
        // without waiting for it to complete
        let mut client = connect_to_daemon().await?;
        let (_start_task, _progress_rx, _) = client.health_check(&key_name, recycle).await?;

        // Don't await the start_task, just let it run in the background

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;
    let (start_task, progress_rx, _) = client.health_check(&key_name, recycle).await?;

    // Create the progress bar wrapper
    // Keep it in scope until the end of the function to ensure progress bars are properly cleaned up
    let _progress = if !quiet {
        let progress = ProgressWrapper::new();
        callbacks::health_check::create_health_check_progress(progress_rx, progress.multi_progress());
        Some(progress)
    } else {
        None
    };

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                if error.contains("not found") || error.contains("No pads found for key") {
                    eprintln!("{} Key '{}' not found.", "Error:".bright_red(), key_name);
                } else {
                    eprintln!("{} {}", "Error:".bright_red(), error);
                }
            }
            TaskResult::Result(result) => match result {
                TaskResultType::HealthCheck(result) => {
                    println!("Health check complete.");
                    println!("  {} keys reset", result.nb_keys_reset);
                    println!("  {} keys recycled", result.nb_keys_recycled);
                }
                _ => {
                    eprintln!("{} {}", "Error:".bright_red(), "Unknown task result");
                }
            },
            TaskResult::Pending => {
                println!("{} Health check task pending.", "•".bright_yellow());
            }
        },
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    Ok(())
}
