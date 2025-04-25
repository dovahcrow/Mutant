use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
use mutant_client::MutantClient;

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
        #[arg(short, long)]
        background: bool,
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

async fn handle_put(key: String, file: String, background: bool) -> Result<()> {
    let data = std::fs::read(&file)?;

    if background {
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) = client.put(&key, &data).await.unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    let (start_task, progress_rx) = client.put(&key, &data).await?;

    let multi_progress = MultiProgress::new();
    callbacks::put::create_put_progress(progress_rx, multi_progress.clone());

    match start_task.await {
        Ok(result) => {
            if let Some(error) = result.error {
                eprintln!("{} {}", "Error:".bright_red(), error);
            } else {
                println!("{} Upload complete!", "•".bright_green());
            }
        }
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    // Keep the MultiProgress instance alive until the task is complete
    drop(multi_progress);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Put {
            key,
            file,
            background,
        } => {
            handle_put(key, file, background).await?;
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
                    let task = client.query_task(task_id).await?;

                    println!(
                        "{} {} - {} ({})",
                        "•".bright_green(),
                        task.id,
                        format!("{:?}", task.task_type).bright_blue(),
                        format!("{:?}", task.status).bright_yellow()
                    );

                    if let Some(progress) = task.progress {
                        println!("  Progress: {:?}", progress);
                    }

                    if let Some(result) = task.result {
                        if let Some(error) = result.error {
                            println!("  {}: {}", "Error".bright_red(), error);
                        } else if let Some(data) = result.data {
                            println!("  {}: {} bytes", "Data".bright_green(), data.len());
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
