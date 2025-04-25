use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use mutant_client::MutantClient;

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

                println!(
                    "{} Task {} queried",
                    "•".bright_green(),
                    task_id.to_string().bright_blue()
                );
            }
        },
    }

    Ok(())
}
