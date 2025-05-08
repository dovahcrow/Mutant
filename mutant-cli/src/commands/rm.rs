use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;

pub async fn handle_rm(key: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;

    match client.rm(&key).await {
        Ok(_) => {
            println!("{} Key '{}' removed.", "•".bright_green(), key);
        }
        Err(e) => {
            if e.to_string().contains("Key not found") {
                eprintln!("{} Key '{}' not found.", "Error:".bright_red(), key);
            } else {
                eprintln!("{} {}", "Error:".bright_red(), e);
            }
            // Don't propagate the error to allow the CLI to exit cleanly
        }
    }

    Ok(())
}
