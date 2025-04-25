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
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = MutantClient::new();
    client.connect("ws://localhost:3012").await?;

    match cli.command {
        Commands::List => {
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
    }

    Ok(())
}
