// use chrono::{DateTime, Utc};
use humansize::{BINARY, format_size};
use log::{debug, info};

use mutant_lib::MutAnt;
use nu_ansi_term::Color;
use std::process::ExitCode;

pub async fn handle_stats(mutant: MutAnt) -> ExitCode {
    debug!("CLI: Handling Stats command");
    info!("Fetching stats...");
    let stats = mutant.get_storage_stats().await;
    println!("Storage Statistics:");
    println!("-------------------");

    println!("Total Keys: {}", stats.nb_keys);
    println!("Total Pads Managed:    {}", stats.total_pads);
    println!("  Occupied (Private):  {}", stats.occupied_pads);
    println!("  Free Pads:           {}", stats.free_pads);
    println!("  Pending Verify Pads: {}", stats.pending_verification_pads);

    ExitCode::SUCCESS
}

fn format_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

fn create_text_gauge(percentage: f64, width: usize, color: Color) -> (String, String, Color) {
    let filled_width = ((percentage / 100.0) * width as f64).round() as usize;
    let empty_width = width.saturating_sub(filled_width);
    let gauge = format!("{:=<1$}{:->2$}", "", filled_width, empty_width);
    let text = format!("{:>6.2}%", percentage);
    (gauge, text, color)
}
