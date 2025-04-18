use humansize::{BINARY, format_size};
use log::{debug, info};
use mutant_lib::MutAnt;
use mutant_lib::storage::KeyDetails;
use std::process::ExitCode;

pub async fn handle_ls(mutant: MutAnt, long: bool) -> ExitCode {
    debug!("CLI: Handling Ls command, long={}", long);

    if long {
        info!("Fetching detailed key/upload information...");
        match mutant.list_key_details().await {
            Ok(details) => {
                if details.is_empty() {
                    println!("No keys or public uploads stored.");
                } else {
                    let mut details = details;
                    details.sort_by(|a, b| a.key.cmp(&b.key));

                    let max_size_width = details
                        .iter()
                        .map(|d| format_size(d.size, BINARY).len())
                        .max()
                        .unwrap_or(8);
                    let max_type_width = 7;

                    println!(
                        "{:<width$} {:<type_width$} {:<12} KEY/NAME",
                        "SIZE",
                        "TYPE",
                        "MODIFIED",
                        width = max_size_width,
                        type_width = max_type_width
                    );

                    for detail in details {
                        let size_str = format_size(detail.size, BINARY);
                        let date_str = detail.modified.format("%b %d %H:%M").to_string();

                        let (type_str, key_display) = if let Some(addr) = detail.public_address {
                            ("Public", format!("{} @ {}", detail.key, addr))
                        } else {
                            let display = if detail.is_finished {
                                detail.key.clone()
                            } else if let Some(percentage) = detail.completion_percentage {
                                format!("*{} ({:.1}%)", detail.key, percentage)
                            } else {
                                format!("*{}", detail.key)
                            };
                            ("Private", display)
                        };

                        println!(
                            "{:<width$} {:<type_width$} {:<12} {}",
                            size_str,
                            type_str,
                            date_str,
                            key_display,
                            width = max_size_width,
                            type_width = max_type_width
                        );
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error fetching details: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        info!("Fetching key/upload summary...");
        match mutant.list_keys().await {
            Ok(summaries) => {
                if summaries.is_empty() {
                    println!("No keys or public uploads stored.");
                } else {
                    let mut summaries = summaries;
                    summaries.sort_by(|a, b| a.name.cmp(&b.name));

                    for summary in summaries {
                        if summary.is_public {
                            if let Some(addr) = summary.address {
                                println!("{} (public @ {})", summary.name, addr);
                            } else {
                                println!("{} (public, address missing)", summary.name);
                            }
                        } else {
                            match mutant.get_key_details(&summary.name).await {
                                Ok(Some(detail)) => {
                                    if detail.is_finished {
                                        println!("{}", summary.name);
                                    } else if let Some(percentage) = detail.completion_percentage {
                                        println!("*{} ({:.1}%)", summary.name, percentage);
                                    } else {
                                        println!("*{}", summary.name);
                                    }
                                }
                                Ok(None) => {
                                    eprintln!(
                                        "Warning: Index inconsistency for key '{}'. Details not found.",
                                        summary.name
                                    );
                                    println!("{}", summary.name);
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Warning: Could not fetch details for key '{}': {}",
                                        summary.name, e
                                    );
                                    println!("{}", summary.name);
                                }
                            }
                        }
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error fetching key/upload summary: {}", e);
                ExitCode::FAILURE
            }
        }
    }
}
