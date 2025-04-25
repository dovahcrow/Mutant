use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use mutant_client::MutantClient;
use tokio::time::{sleep, Duration};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3030/ws").await?;

    // Give the connection a moment to fully establish
    sleep(Duration::from_millis(100)).await;

    match cli.command {
        Commands::List => {
            let tasks = client.list_tasks().await?;
            println!("{:#?}", tasks);

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
    }

    // Keep the connection alive for a moment to ensure the response is received
    sleep(Duration::from_secs(1)).await;

    Ok(())
}
