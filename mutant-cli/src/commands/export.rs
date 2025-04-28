use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;

pub async fn handle_export(destination_path: String) -> Result<()> {
    let mut client = connect_to_daemon().await?;
    client.export(&destination_path).await?;
    println!(
        "{} Exported file '{}'.",
        "•".bright_green(),
        destination_path
    );
    Ok(())
}
