use crate::callbacks;
use crate::connect_to_daemon;
use crate::terminal::ProgressWithDisabledStdin;
use anyhow::Result;
use colored::Colorize;
use mutant_protocol::TaskResult;
use mutant_protocol::TaskResultType;

pub async fn handle_sync(background: bool, push_force: bool, quiet: bool) -> Result<()> {
    if background {
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) = client.sync(push_force).await.unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;
    let (start_task, progress_rx) = client.sync(push_force).await?;

    // Create the progress bar wrapper that will disable stdin
    // Keep it in scope until the end of the function to ensure stdin remains disabled
    // and progress bars are properly cleaned up
    let _progress = if !quiet {
        let progress = ProgressWithDisabledStdin::new();
        callbacks::sync::create_sync_progress(progress_rx, progress.multi_progress());
        Some(progress)
    } else {
        None
    };

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                eprintln!("{} {}", "Error:".bright_red(), error);
            }
            TaskResult::Result(result) => match result {
                TaskResultType::Sync(result) => {
                    println!("Synchronization complete.");
                    println!("  {} keys added", result.nb_keys_added);
                    println!("  {} keys updated", result.nb_keys_updated);
                    println!("  {} free pads added", result.nb_free_pads_added);
                    println!("  {} pending pads added", result.nb_pending_pads_added);
                    println!("{} Sync task completed.", "•".bright_green());
                }
                _ => {
                    eprintln!("{} {}", "Error:".bright_red(), "Unknown task result");
                }
            },
            TaskResult::Pending => {
                println!("{} Sync task pending.", "•".bright_yellow());
            }
        },
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    Ok(())
}
