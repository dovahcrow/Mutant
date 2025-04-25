use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
use log::{error, info, warn};
use mutant_client::MutantClient;
use mutant_protocol::TaskProgress;

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

    let (start_task, progress_rx) = client.put(&key, &data).await?;

    callbacks::put::create_put_progress(progress_rx);

    match start_task.await {
        Ok(result) => {
            if let Some(error) = result.error {
                error!("Upload failed: {}", error);
                eprintln!("{} {}", "Error:".bright_red(), error);
            } else {
                info!("Upload completed successfully");
                println!("{} Upload complete!", "•".bright_green());
            }
        }
        Err(e) => {
            error!("Task failed: {:?}", e);
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Put { key, file } => {
            handle_put(key, file).await?;
        }
        Commands::Tasks { command } => {
            let mut client = connect_to_daemon().await?;

            match command {
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
                }
            }
        }
    }

    Ok(())
}
