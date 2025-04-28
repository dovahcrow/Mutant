use crate::callbacks;
use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;
use indicatif::MultiProgress;
use mutant_protocol::StorageMode;
use mutant_protocol::TaskResult;

pub async fn handle_put(
    key: String,
    file: String,
    public: bool,
    mode: StorageMode,
    no_verify: bool,
    background: bool,
) -> Result<()> {
    let source_path = file;
    if background {
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) = client
                .put(&key, &source_path, mode, public, no_verify)
                .await
                .unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    let (start_task, progress_rx) = client
        .put(&key, &source_path, mode, public, no_verify)
        .await?;

    let multi_progress = MultiProgress::new();
    callbacks::put::create_put_progress(progress_rx, multi_progress.clone());

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                eprintln!("{} {}", "Error:".bright_red(), error);
            }
            TaskResult::Result(_result) => {
                println!("{} Upload complete!", "•".bright_green());
            }
            TaskResult::Pending => {
                println!("{} Upload pending.", "•".bright_yellow());
            }
        },
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    // Keep the MultiProgress instance alive until the task is complete
    drop(multi_progress);

    Ok(())
}
