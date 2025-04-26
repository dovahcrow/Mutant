use anyhow::Result;
use clap::Parser;
use clap::ValueEnum;
use colored::Colorize;
use humansize::{format_size, BINARY};
use indicatif::MultiProgress;
use mutant_client::MutantClient;
use mutant_protocol::KeyDetails;
use mutant_protocol::StatsResponse;
use mutant_protocol::StorageMode;

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
        public: bool,
        #[arg(value_enum, short, long, default_value_t = StorageModeCli::Medium)]
        mode: StorageModeCli,
        #[arg(short, long)]
        background: bool,
        #[arg(short, long)]
        no_verify: bool,
    },
    Get {
        key: String,
        destination_path: String,
        #[arg(short, long)]
        background: bool,
        #[arg(short, long)]
        public: bool,
    },
    Rm {
        key: String,
    },
    #[command(about = "List stored keys")]
    Ls,
    #[command(about = "Show storage statistics")]
    Stats,
    Tasks {
        #[command(subcommand)]
        command: TasksCommands,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum StorageModeCli {
    /// 0.5 MB per scratchpad
    Lightest,
    /// 1 MB per scratchpad
    Light,
    /// 2 MB per scratchpad
    Medium,
    /// 3 MB per scratchpad
    Heavy,
    /// 4 MB per scratchpad
    Heaviest,
}

impl From<StorageModeCli> for StorageMode {
    fn from(mode: StorageModeCli) -> Self {
        match mode {
            StorageModeCli::Lightest => StorageMode::Lightest,
            StorageModeCli::Light => StorageMode::Light,
            StorageModeCli::Medium => StorageMode::Medium,
            StorageModeCli::Heavy => StorageMode::Heavy,
            StorageModeCli::Heaviest => StorageMode::Heaviest,
        }
    }
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

async fn handle_put(
    key: String,
    file: String,
    public: bool,
    mode: StorageMode,
    no_verify: bool,
    background: bool,
) -> Result<()> {
    let source_path = file;
    if background {
        let _ = tokio::spawn(async move {
            let mut client = connect_to_daemon().await.unwrap();
            let (start_task, _progress_rx) = client
                .put(&key, &source_path, mode, public, no_verify)
                .await
                .unwrap();
            start_task.await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        return Ok(());
    }

    let mut client = connect_to_daemon().await?;

    let (start_task, progress_rx) = client
        .put(&key, &source_path, mode, public, no_verify)
        .await?;

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

async fn handle_get(
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
        Ok(result) => {
            if let Some(error) = result.error {
                eprintln!("{} {}", "Error:".bright_red(), error);
            } else {
                println!(
                    "{} Get task completed. Result saved to {} on daemon.",
                    "•".bright_green(),
                    destination_path
                );
            }
        }
        Err(e) => {
            eprintln!("{} Task failed: {}", "Error:".bright_red(), e);
        }
    }

    Ok(())
}

async fn handle_rm(key: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;
    client.rm(&key).await?;
    println!("{} Key '{}' removed.", "•".bright_green(), key);
    Ok(())
}

async fn handle_ls() -> Result<()> {
    let mut client = connect_to_daemon().await?;
    let details = client.list_keys().await?;

    if details.is_empty() {
        println!("No keys stored.");
    } else {
        println!(
            "{:<20} {:>5} {:>10} {:<12} {}",
            "Key", "Pads", "Size", "Status", "Address/Info"
        );
        println!("{}", "-".repeat(70));

        for detail in details {
            let completion_str = if detail.pad_count == 0 {
                "0% (0/0)".to_string()
            } else if detail.confirmed_pads == detail.pad_count {
                "Ready".bright_green().to_string()
            } else {
                format!(
                    "{}% ({}/{})",
                    detail.confirmed_pads * 100 / detail.pad_count,
                    detail.confirmed_pads,
                    detail.pad_count
                )
                .bright_yellow()
                .to_string()
            };

            let size_str = format_size(detail.total_size, BINARY);

            let address_info = if detail.is_public {
                format!("Public: {}", detail.public_address.unwrap_or_default())
            } else {
                "Private".to_string()
            };

            println!(
                "{:<20} {:>5} {:>10} {:<21} {}",
                detail.key,
                detail.pad_count,
                size_str,
                completion_str.to_string(),
                address_info
            );
        }
    }
    Ok(())
}

async fn handle_stats() -> Result<()> {
    let mut client = connect_to_daemon().await?;
    let stats = client.get_stats().await?;

    println!("Storage Statistics:");
    println!("-------------------");
    println!("Total Keys: {}", stats.total_keys);
    println!("Total Pads Managed:    {}", stats.total_pads);
    println!("  Occupied (Private):  {}", stats.occupied_pads);
    println!("  Free Pads:           {}", stats.free_pads);
    println!("  Pending Verify Pads: {}", stats.pending_verify_pads);

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
            no_verify,
            public,
            mode,
        } => {
            handle_put(key, file, public, mode.into(), no_verify, background).await?;
        }
        Commands::Get {
            key,
            destination_path,
            background,
            public,
        } => {
            handle_get(key, destination_path, public, background).await?;
        }
        Commands::Rm { key } => {
            handle_rm(key).await?;
        }
        Commands::Ls => {
            handle_ls().await?;
        }
        Commands::Stats => {
            handle_stats().await?;
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

                    println!("Task: {:#?}", task);

                    println!(
                        "{} {} - {} ({})",
                        "•".bright_green(),
                        task.id,
                        format!("{:?}", task.task_type).bright_blue(),
                        format!("{:?}", task.status).bright_yellow()
                    );

                    if let Some(progress) = task.progress {
                        match progress {
                            mutant_protocol::TaskProgress::Put(event) => {
                                println!("  Progress: {:?}", event);
                            }
                            mutant_protocol::TaskProgress::Get(event) => {
                                println!("  Progress: {:?}", event);
                            }
                        }
                    }

                    if let Some(result) = task.result {
                        if let Some(error) = result.error {
                            println!("  {}: {}", "Error".bright_red(), error);
                        } else {
                            println!(
                                "  {}: Completed (result stored on daemon)",
                                "Result".bright_green()
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
