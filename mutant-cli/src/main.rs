use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
use log::{debug, error, info, warn};
use mutant_client::MutantClient;
use mutant_protocol::{PutEvent, Response, TaskProgress, TaskStatus};
use std::sync::Arc;
use tokio::sync::mpsc;

mod callbacks;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    Put {
        key: String,
        file: String,
    },
    Tasks {
        #[command(subcommand)]
        command: TasksCommands,
    },
}

#[derive(clap::Subcommand)]
enum TasksCommands {
    List,
    Get { task_id: String },
}

async fn connect_to_daemon() -> Result<MutantClient> {
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3030/ws").await?;
    Ok(client)
}

async fn handle_put(key: String, file: String) -> Result<()> {
    let data = std::fs::read(&file)?;

    let mut client = connect_to_daemon().await?;

    let (task, progress_rx) = client.put(&key, &data).await?;

    create_put_callback(progress_rx);

    task.await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let mut client = MutantClient::new();

    client.connect("ws://localhost:3030/ws").await?;

    match cli.command {
        Commands::Put { key, file } => {
            info!(
                "Starting put operation for key '{}' from file '{}'",
                key, file
            );
            let data = std::fs::read(&file)?;
            info!("Read {} bytes from file", data.len());

            let multi_progress = MultiProgress::new();
            info!("Creating progress bars");
            let (_res_pb_opt, _upload_pb_opt, _confirm_pb_opt, callback) =
                callbacks::put::create_put_callback(&multi_progress, false);
            info!("Progress bars created");

            let mut client_clone = client.clone();
            let task_id = Arc::new(tokio::sync::Mutex::new(None));
            let task_id_clone = task_id.clone();

            info!("Starting response monitoring task");
            let handle = tokio::spawn(async move {
                info!("Response monitoring loop started");
                while let Some(response) = client_clone.next_response().await {
                    match response {
                        Ok(response) => {
                            let current_task_id =
                                if let Some(id) = task_id_clone.lock().await.as_ref() {
                                    *id
                                } else {
                                    debug!("No task ID set yet, skipping response");
                                    continue;
                                };

                            match &response {
                                Response::TaskUpdate(update) => {
                                    if update.task_id == current_task_id {
                                        info!("Received task update: {:?}", update);
                                        if let Some(progress) = &update.progress {
                                            match progress {
                                                TaskProgress::Put(event) => {
                                                    info!("Processing Put event: {:?}", event);
                                                    if let Err(e) = callback(event.clone()).await {
                                                        error!(
                                                            "Error processing put event: {:?}",
                                                            e
                                                        );
                                                    }
                                                    debug!("Put event processed");
                                                }
                                                _ => {
                                                    warn!(
                                                        "Unexpected progress type: {:?}",
                                                        progress
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                Response::TaskResult(result) => {
                                    if result.task_id == current_task_id {
                                        info!("Received task result: {:?}", result);
                                        match result.status {
                                            TaskStatus::Completed => {
                                                info!("Upload completed successfully");
                                                println!("{} Upload complete!", "•".bright_green());
                                                break;
                                            }
                                            TaskStatus::Failed => {
                                                if let Some(result) = &result.result {
                                                    if let Some(error) = &result.error {
                                                        error!("Upload failed: {}", error);
                                                        eprintln!(
                                                            "{} {}",
                                                            "Error:".bright_red(),
                                                            error
                                                        );
                                                    }
                                                }
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                _ => debug!("Received other response type: {:?}", response),
                            }
                        }
                        Err(e) => {
                            error!("Error receiving response: {:?}", e);
                            eprintln!("{} Error receiving response: {}", "Error:".bright_red(), e);
                            break;
                        }
                    }
                }
                info!("Response monitoring loop ended");
            });

            info!("Sending put request");
            let new_task_id = client.put(&key, &data).await?;
            info!("Put request sent, got task ID: {}", new_task_id);
            *task_id.lock().await = Some(new_task_id);

            println!(
                "{} Task created with id: {}",
                "•".bright_green(),
                new_task_id.to_string().bright_blue()
            );

            info!("Waiting for task completion");
            if let Err(e) = handle.await {
                error!("Task monitoring failed: {:?}", e);
                eprintln!("{} Task monitoring failed: {}", "Error:".bright_red(), e);
            }
            info!("Task completed");
        }
        Commands::Tasks { command } => match command {
            TasksCommands::List => {
                let tasks = client.list_tasks().await?;

                for task in tasks {
                    println!(
                        "{} {} - {} ({})",
                        "•".bright_green(),
                        task.task_id,
                        format!("{:?}", task.task_type).bright_blue(),
                        format!("{:?}", task.status).bright_yellow()
                    );
                }
            }
            TasksCommands::Get { task_id } => {
                let task_id = uuid::Uuid::parse_str(&task_id)?;
                client.query_task(task_id).await?;

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                let status = client.get_task_status(task_id);
                let result = client.get_task_result(task_id);

                println!(
                    "{} Task {} queried:",
                    "•".bright_green(),
                    task_id.to_string().bright_blue()
                );

                if let Some(status) = status {
                    println!("  Status: {}", format!("{:?}", status).bright_yellow());
                }

                if let Some(result) = result {
                    if let Some(error) = result.error {
                        println!("  Error: {}", error.bright_red());
                    }
                    if let Some(data) = result.data {
                        println!("  Data: {}", data.bright_green());
                    }
                }
            }
        },
    }

    Ok(())
}
