use humansize::{BINARY, format_size};
use log::{debug, info};

use mutant_lib::MutAnt;
use std::process::ExitCode;

pub async fn handle_ls(mutant: MutAnt, long: bool) -> ExitCode {
    debug!("CLI: Handling Ls command, long={}", long);

    if long {
        info!("Fetching key details...");
        match mutant.list_key_details().await {
            Ok(details) => {
                if details.is_empty() {
                    println!("No keys stored.");
                } else {
                    let mut details = details;
                    details.sort_by(|a, b| a.key.cmp(&b.key));

                    let max_size_width = details
                        .iter()
                        .map(|d| format_size(d.size, BINARY).len())
                        .max()
                        .unwrap_or(0);

                    println!(
                        "{:<width$} {:<12} KEY",
                        "SIZE",
                        "MODIFIED",
                        width = max_size_width
                    );

                    for detail in details {
                        let size_str = format_size(detail.size, BINARY);
                        let date_str = detail.modified.format("%b %d %H:%M").to_string();
                        let key_display = if detail.is_finished {
                            detail.key.clone()
                        } else if let Some(percentage) = detail.completion_percentage {
                            format!("*{} ({:.1}%)", detail.key, percentage)
                        } else {
                            format!("*{}", detail.key)
                        };
                        println!(
                            "{:<width$} {:<12} {}",
                            size_str,
                            date_str,
                            key_display,
                            width = max_size_width
                        );
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error fetching key details: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        info!("Fetching key details (for status markers)...");

        match mutant.list_key_details().await {
            Ok(details) => {
                if details.is_empty() {
                    println!("No keys stored.");
                } else {
                    let mut details = details;

                    details.sort_by(|a, b| a.key.cmp(&b.key));
                    for detail in details {
                        if detail.is_finished {
                            println!("{}", detail.key);
                        } else if let Some(percentage) = detail.completion_percentage {
                            println!("*{} ({:.1}%)", detail.key, percentage);
                        } else {
                            println!("*{}", detail.key);
                        }
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error fetching key list/details: {}", e);
                ExitCode::FAILURE
            }
        }
    }
}
