use crate::callbacks::{create_get_callback, create_put_callback};
use crate::cli::Commands;

use humansize::{BINARY, format_size};
use indicatif::MultiProgress;
use log::{debug, error, info, warn};
use mutant_lib::{error::Error, mutant::MutAnt};
use nu_ansi_term::{Color, Style};
use std::io::{self, Read, Write};
use std::process::ExitCode;

pub async fn handle_command(
    command: Commands,
    mutant: MutAnt,
    multi_progress: &MultiProgress,
) -> ExitCode {
    match command {
        Commands::Put { key, value, force } => {
            handle_put(mutant, key, value, force, multi_progress).await
        }
        Commands::Get { key } => handle_get(mutant, key, multi_progress).await,
        Commands::Rm { key } => handle_rm(mutant, key).await,
        Commands::Ls { long } => handle_ls(mutant, long).await,
        Commands::Stats => handle_stats(mutant).await,
        Commands::Reset => {
            error!("Internal error: Reset command reached handle_command");
            unreachable!("Reset command should be handled before handle_command");
        }
    }
}

async fn handle_put(
    mutant: MutAnt,
    key: String,
    value: Option<String>,
    force: bool,
    multi_progress: &MultiProgress,
) -> ExitCode {
    debug!(
        "CLI: Handling Put command: key={}, value provided: {}, force: {}",
        key,
        value.is_some(),
        force
    );
    let bytes_to_store = match value {
        Some(v) => v.into_bytes(),
        None => {
            let mut buffer = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut buffer) {
                eprintln!("Error reading value from stdin: {}", e);
                return ExitCode::FAILURE;
            }
            buffer
        }
    };

    let put_multi_progress = multi_progress.clone();
    let (reservation_pb, upload_pb, _res_confirmed, _last_progress, callback) =
        create_put_callback(&put_multi_progress);

    let result = if force {
        debug!("Forcing update for key: {}", key);
        mutant
            .update_with_progress(&key, &bytes_to_store, Some(callback))
            .await
    } else {
        mutant
            .store_with_progress(&key, &bytes_to_store, Some(callback))
            .await
    };

    match result {
        Ok(_) => {
            debug!("handle_put: Result Ok. Attempting multi_progress.clear()...");
            if let Err(e) = multi_progress.clear() {
                warn!("Failed to clear multi-progress bars: {}", e);
            }
            debug!("handle_put: multi_progress.clear() finished. Returning ExitCode::SUCCESS...");
            ExitCode::SUCCESS
        }
        Err(Error::OperationCancelled) => {
            eprintln!("Operation cancelled.");
            if let Some(pb) = reservation_pb.lock().unwrap().take() {
                if !pb.is_finished() {
                    pb.finish_and_clear();
                }
            }
            if let Some(pb) = upload_pb.lock().unwrap().take() {
                if !pb.is_finished() {
                    pb.finish_and_clear();
                }
            }
            ExitCode::FAILURE
        }
        Err(Error::KeyAlreadyExists(_)) if !force => {
            eprintln!("Key '{}' already exists. Use --force to overwrite.", key);
            ExitCode::FAILURE
        }
        Err(Error::KeyNotFound(_)) if force => {
            eprintln!(
                "Key not found for forced update: {}. Use put without --force to create.",
                key
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!(
                "Error during {}: {}",
                if force { "update" } else { "store" },
                e
            );
            let fail_msg = "Operation Failed".to_string();
            if let Some(pb) = (*reservation_pb.lock().unwrap()).take() {
                if !pb.is_finished() {
                    pb.abandon_with_message(fail_msg.clone());
                }
            }
            if let Some(pb) = (*upload_pb.lock().unwrap()).take() {
                if !pb.is_finished() {
                    pb.abandon_with_message(fail_msg.clone());
                }
            }
            ExitCode::FAILURE
        }
    }
}

async fn handle_get(mutant: MutAnt, key: String, multi_progress: &MultiProgress) -> ExitCode {
    debug!("CLI: Handling Get command: key={}", key);

    let get_multi_progress = multi_progress.clone();
    let (download_pb_opt, callback) = create_get_callback(&get_multi_progress);

    match mutant.fetch_with_progress(&key, Some(callback)).await {
        Ok(value_bytes) => {
            if let Some(pb) = download_pb_opt.lock().unwrap().take() {
                if !pb.is_finished() {
                    pb.finish_and_clear();
                    debug!("handle_get: Success - Finishing and clearing progress bar.");
                }
            }

            if let Err(e) = io::stdout().write_all(&value_bytes) {
                eprintln!("Error writing fetched data to stdout: {}", e);
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            let error_message = format!("Error getting data: {}", e);
            eprintln!("{}", error_message);

            if let Some(pb) = download_pb_opt.lock().unwrap().take() {
                if !pb.is_finished() {
                    pb.abandon_with_message(error_message);
                    debug!("handle_get: Error - Abandoning progress bar.");
                } else {
                    debug!("handle_get: Error - Progress bar already finished.");
                }
            }
            ExitCode::FAILURE
        }
    }
}

async fn handle_rm(mutant: MutAnt, key: String) -> ExitCode {
    debug!("CLI: Handling Rm command: key={}", key);
    match mutant.remove(&key).await {
        Ok(_) => {
            println!("Removed key: {}", key);
            ExitCode::SUCCESS
        }
        Err(Error::KeyNotFound(_)) => {
            eprintln!("Key not found: {}", key);
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("Error removing key: {}", e);
            ExitCode::FAILURE
        }
    }
}

async fn handle_ls(mutant: MutAnt, long: bool) -> ExitCode {
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
                        println!(
                            "{:<width$} {:<12} {}",
                            size_str,
                            date_str,
                            detail.key,
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
            Ok(keys) => {
                let mut keys = keys;
                keys.sort();

                if keys.is_empty() {
                    println!("No keys stored.");
                } else {
                    for key in keys {
                        println!("{}", key);
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

async fn handle_stats(mutant: MutAnt) -> ExitCode {
    debug!("CLI: Handling Stats command");
    info!("Fetching stats...");

    const LABEL_WIDTH: usize = 64;
    const GAUGE_BAR_WIDTH: usize = 22;
    const PERCENTAGE_WIDTH: usize = 7;

    match mutant.get_storage_stats().await {
        Ok(stats) => {
            let label_style = Style::new().bold();
            let highlight_style = Style::new().fg(Color::Yellow);

            let capacity_pads_percentage = if stats.total_pads > 0 {
                stats.occupied_pads as f64 / stats.total_pads as f64
            } else {
                0.0
            };
            let data_storage_percentage = if stats.total_space_bytes > 0 {
                stats.occupied_data_bytes as f64 / stats.total_space_bytes as f64
            } else {
                0.0
            };

            let (capacity_bar, capacity_percent, capacity_color) =
                create_text_gauge(capacity_pads_percentage, 20, Color::Green);
            let colored_gauge_bar = Style::new().fg(capacity_color).paint(&capacity_bar);
            let capacity_info = highlight_style.paint(format!(
                "{} / {} pads",
                stats.occupied_pads, stats.total_pads
            ));

            println!(
                "{label:<label_width$}       {gauge_bar:<gauge_width$} {percentage:>percent_width$} {info}",
                label = label_style.paint("Capacity"),
                gauge_bar = colored_gauge_bar,
                percentage = capacity_percent,
                info = capacity_info,
                label_width = LABEL_WIDTH,
                gauge_width = GAUGE_BAR_WIDTH,
                percent_width = PERCENTAGE_WIDTH
            );

            let (data_bar, data_percent, data_color) =
                create_text_gauge(data_storage_percentage, 20, Color::Blue);
            let colored_gauge_bar = Style::new().fg(data_color).paint(&data_bar);
            let data_info = highlight_style.paint(format!(
                "{} / {}",
                format_bytes(stats.occupied_data_bytes),
                format_bytes(stats.total_space_bytes)
            ));

            println!(
                "{label:<label_width$}     {gauge_bar:<gauge_width$} {percentage:>percent_width$} {info}",
                label = label_style.paint("Data Usage"),
                gauge_bar = colored_gauge_bar,
                percentage = data_percent,
                info = data_info,
                label_width = LABEL_WIDTH,
                gauge_width = GAUGE_BAR_WIDTH,
                percent_width = PERCENTAGE_WIDTH
            );

            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error fetching stats: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNIT: u64 = 1024;
    if bytes < UNIT {
        return format!("{} B", bytes);
    }
    let mut exp = (bytes as f64).log(UNIT as f64).floor() as u32;
    if exp > 4 {
        exp = 4;
    }
    let pre = "KMGT".chars().nth((exp - 1) as usize).unwrap();
    let value = bytes as f64 / (UNIT as f64).powi(exp as i32);
    format!("{:.2} {}B", value, pre)
}

fn create_text_gauge(percentage: f64, width: usize, color: Color) -> (String, String, Color) {
    let filled_width = (percentage * width as f64).round() as usize;
    let empty_width = width.saturating_sub(filled_width);

    let filled_char = '#';
    let empty_char = '-';

    let gauge_bar = format!(
        "[{}{}]",
        filled_char.to_string().repeat(filled_width),
        empty_char.to_string().repeat(empty_width)
    );

    let percentage_text = format!("{:>6.2}%", percentage * 100.0);

    (gauge_bar, percentage_text, color)
}
