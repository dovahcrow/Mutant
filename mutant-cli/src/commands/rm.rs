use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;

pub async fn handle_rm(key: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;
    client.rm(&key).await?;
    println!("{} Key '{}' removed.", "•".bright_green(), key);
    Ok(())
}
