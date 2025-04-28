use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;

pub async fn handle_import(file_path: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;
    client.import(&file_path).await?;
    println!("{} Imported file '{}'.", "•".bright_green(), file_path);
    Ok(())
}
