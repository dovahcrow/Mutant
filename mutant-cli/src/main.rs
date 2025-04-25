use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use mutant_client::MutantClient;
use mutant_protocol::{PutProgressEvent, Response, TaskProgress, TaskStatus, TaskType};
use std::sync::Arc;
use tokio::sync::mpsc;

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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3030/ws").await?;

    match cli.command {
        Commands::Put { key, file } => {
            let data = std::fs::read(&file)?;
            let task_id = client.put(&key, &data).await?;

            println!(
                "{} Task created with id: {}",
                "•".bright_green(),
                task_id.to_string().bright_blue()
            );

            let pb = ProgressBar::new(100);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}% {msg}",
                    )
                    .unwrap()
                    .progress_chars("#>-"),
            );

            let (tx, mut rx) = mpsc::channel(100);
            let mut client_clone = client.clone();
            let pb = Arc::new(pb);
            let pb_clone = pb.clone();

            tokio::spawn(async move {
                while let Some(response) = client_clone.next_response().await {
                    if let Ok(response) = response {
                        if let Response::TaskUpdate(update) = response {
                            if update.task_id == task_id {
                                if let Some(progress) = update.progress {
                                    match progress {
                                        TaskProgress::Legacy { message } => {
                                            let _ = tx.send(message).await;
                                        }
                                        TaskProgress::Put(event) => {
                                            let message = match event {
                                                PutProgressEvent::Starting {
                                                    total_chunks,
                                                    initial_written_count,
                                                    initial_confirmed_count,
                                                    chunks_to_reserve,
                                                } => {
                                                    format!("Starting upload: {} chunks total, {} written, {} confirmed, {} to reserve", total_chunks, initial_written_count, initial_confirmed_count, chunks_to_reserve)
                                                }
                                                PutProgressEvent::PadReserved => {
                                                    "Pad space reserved".to_string()
                                                }
                                                PutProgressEvent::PadsWritten => {
                                                    "Pads written".to_string()
                                                }
                                                PutProgressEvent::PadsConfirmed => {
                                                    "Pads confirmed".to_string()
                                                }
                                                PutProgressEvent::Complete => {
                                                    "Upload complete".to_string()
                                                }
                                            };
                                            let _ = tx.send(message).await;
                                        }
                                    }
                                }
                            }
                        } else if let Response::TaskResult(result) = response {
                            if result.task_id == task_id {
                                match result.status {
                                    TaskStatus::Completed => {
                                        pb_clone.finish_with_message("Upload complete!");
                                        break;
                                    }
                                    TaskStatus::Failed => {
                                        pb_clone.abandon_with_message("Upload failed!");
                                        if let Some(result) = result.result {
                                            if let Some(error) = result.error {
                                                eprintln!("{} {}", "Error:".bright_red(), error);
                                            }
                                        }
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            });

            while let Some(msg) = rx.recv().await {
                pb.set_message(msg);
                pb.inc(10);
            }
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

                // Give a small delay for the response to arrive
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
