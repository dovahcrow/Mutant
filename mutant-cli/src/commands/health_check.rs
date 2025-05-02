use crate::callbacks;
use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;
use indicatif::MultiProgress;
use mutant_protocol::TaskResult;
use mutant_protocol::TaskResultType;

pub async fn handle_health_check(
    key_name: String,
    background: bool,
    recycle: bool,
    quiet: bool,
) -> Result<()> {
    if background {
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) = client.health_check(&key_name, recycle).await.unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;
    let (start_task, progress_rx) = client.health_check(&key_name, recycle).await?;

    if !quiet {
        let multi_progress = MultiProgress::new();
        callbacks::health_check::create_health_check_progress(progress_rx, &multi_progress);
    }

    match start_task.await {
        Ok(result) => match result {
            TaskResult::Error(error) => {
                eprintln!("{} {}", "Error:".bright_red(), error);
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
