use humansize::{BINARY, format_size};
use log::{debug, info};
use mutant_lib::mutant::MutAnt;
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
                        } else {
                            // Display percentage if available
                            if let Some(percentage) = detail.completion_percentage {
                                format!("*{} ({:.1}%)", detail.key, percentage)
                            } else {
                                format!("*{}", detail.key) // Fallback if no percentage
                            }
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
        info!("Fetching key list...");
        match mutant.list_keys().await {
            Ok(keys_with_status) => {
                if keys_with_status.is_empty() {
                    println!("No keys stored.");
                } else {
                    let mut keys_with_status = keys_with_status;
                    // Sort by key
                    keys_with_status.sort_by(|(key_a, _, _), (key_b, _, _)| key_a.cmp(key_b));
                    for (key, is_finished, percentage_opt) in keys_with_status {
                        if is_finished {
                            println!("{}", key);
                        } else {
                            // Display percentage if available
                            if let Some(percentage) = percentage_opt {
                                println!("*{} ({:.1}%)", key, percentage);
                            } else {
                                println!("*{}", key); // Fallback if no percentage
                            }
                        }
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error fetching key list: {}", e);
                ExitCode::FAILURE
            }
        }
    }
}
