use crate::{cli::TasksCommands, connect_to_daemon};
use anyhow::Result;
use colored::Colorize;
use mutant_protocol::{TaskProgress, TaskResult};

pub async fn handle_tasks(command: TasksCommands) -> Result<()> {
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
                    TaskProgress::Put(event) => {
                        println!("  Progress: {:?}", event);
                    }
                    TaskProgress::Get(event) => {
                        println!("  Progress: {:?}", event);
                    }
                    TaskProgress::Sync(event) => {
                        println!("  Progress: {:?}", event);
                    }
                    TaskProgress::Purge(event) => {
                        println!("  Progress: {:?}", event);
                    }
                    TaskProgress::HealthCheck(event) => {
                        println!("  Progress: {:?}", event);
                    }
                    TaskProgress::Colony(event) => {
                        println!("  Progress: {:?}", event);
                    }
                }
            }

            match task.result {
                TaskResult::Pending => {
                    println!("  {}: {}", "Result".bright_yellow(), "Pending");
                }
                TaskResult::Error(error) => {
                    println!("  {}: {}", "Error".bright_red(), error);
                }
                TaskResult::Result(_result) => {
                    println!(
                        "  {}: Completed (result stored on daemon)",
                        "Result".bright_green()
                    );
                }
            }
        }
        TasksCommands::Stop { task_id } => {
            let task_id = uuid::Uuid::parse_str(&task_id)?;
            client.stop_task(task_id).await?;
        }
    }

    Ok(())
}
