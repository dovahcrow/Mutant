use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
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

            let multi_progress = MultiProgress::new();
            let (_res_pb_opt, _upload_pb_opt, _confirm_pb_opt, callback) =
                callbacks::put::create_put_callback(&multi_progress, false);

            let mut client_clone = client.clone();

            tokio::spawn(async move {
                while let Some(response) = client_clone.next_response().await {
                    if let Ok(response) = response {
                        if let Response::TaskUpdate(update) = response {
                            if update.task_id == task_id {
                                if let Some(progress) = update.progress {
                                    match progress {
                                        TaskProgress::Put(event) => {
                                            let _ = callback(event).await;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        } else if let Response::TaskResult(result) = response {
                            if result.task_id == task_id {
                                match result.status {
                                    TaskStatus::Completed => {
                                        println!("{} Upload complete!", "•".bright_green());
                                        break;
                                    }
                                    TaskStatus::Failed => {
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
            })
            .await?;
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
