use crate::connect_to_daemon;
use anyhow::Result;
use colored::Colorize;
use humansize::{format_size, BINARY};

pub async fn handle_ls() -> Result<()> {
    let mut client = connect_to_daemon().await?;
    let details = client.list_keys().await?;

    if details.is_empty() {
        println!("No keys stored.");
    } else {
        println!(
            "{: <20} {:>5} {:>10} {: <12} {}",
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
                "{: <20} {:>5} {:>10} {: <21} {}",
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
