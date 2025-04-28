use crate::callbacks;
use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;
use indicatif::MultiProgress;
use mutant_protocol::TaskResult;

pub async fn handle_get(
    key: String,
    destination_path: String,
    public: bool,
    background: bool,
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
    let (start_task, progress_rx) = client.get(&key, &destination_path, public).await?;

    let multi_progress = MultiProgress::new();
    callbacks::get::create_get_progress(progress_rx, &multi_progress);

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                eprintln!("{} {}", "Error:".bright_red(), error);
            }
            TaskResult::Result(_result) => {
                println!(
                    "{} Get task completed. Result saved to {} on daemon.",
                    "•".bright_green(),
                    destination_path
                );
            }
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
