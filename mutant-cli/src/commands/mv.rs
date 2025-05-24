use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;

pub async fn handle_mv(old_key: String, new_key: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;

    match client.mv(&old_key, &new_key).await {
        Ok(_) => {
            println!("{} Key '{}' renamed to '{}'.", "•".bright_green(), old_key, new_key);
        }
        Err(e) => {
            if e.to_string().contains("not found") {
                eprintln!("{} Key '{}' not found.", "Error:".bright_red(), old_key);
            } else if e.to_string().contains("already exists") {
                eprintln!("{} Key '{}' already exists.", "Error:".bright_red(), new_key);
            } else {
                eprintln!("{} {}", "Error:".bright_red(), e);
            }
            // Don't propagate the error to allow the CLI to exit cleanly
        }
    }

    Ok(())
}
