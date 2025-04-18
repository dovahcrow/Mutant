// use chrono::{DateTime, Utc};
use humansize::{BINARY, format_size};
use log::{debug, info};

use mutant_lib::prelude::MutAnt;
use nu_ansi_term::Color;
use std::process::ExitCode;

pub async fn handle_stats(mutant: MutAnt) -> ExitCode {
    debug!("CLI: Handling Stats command");
    info!("Fetching stats...");
    match mutant.get_storage_stats().await {
        Ok(stats) => {
            println!("Storage Statistics:");
            println!("-------------------");
            println!(
                "Scratchpad Size:       {}",
                format_bytes(stats.scratchpad_size as u64)
            );
            println!("Total Pads:            {}", stats.total_pads);
            println!("  Occupied Pads:       {}", stats.occupied_pads);
            println!("  Free Pads:           {}", stats.free_pads);
            println!("  Pending Verify Pads: {}", stats.pending_verification_pads);
            println!("-------------------");
            println!(
                "Total Space:           {}",
                format_bytes(stats.total_space_bytes)
            );
            println!(
                "  Occupied Pad Space:  {}",
                format_bytes(stats.occupied_pad_space_bytes)
            );
            println!(
                "  Free Pad Space:      {}",
                format_bytes(stats.free_pad_space_bytes)
            );
            println!("-------------------");
            println!(
                "Actual Data Stored:    {}",
                format_bytes(stats.occupied_data_bytes)
            );
            println!(
                "Wasted Space (Frag.):  {}",
                format_bytes(stats.wasted_space_bytes)
            );
            println!("-------------------");

            let occupied_pads_pct = if stats.total_pads > 0 {
                stats.occupied_pads as f64 / stats.total_pads as f64 * 100.0
            } else {
                0.0
            };
            let free_pads_pct = 100.0 - occupied_pads_pct;

            let used_space_pct = if stats.total_space_bytes > 0 {
                stats.occupied_data_bytes as f64 / stats.total_space_bytes as f64 * 100.0
            } else {
                0.0
            };
            let free_space_pct = if stats.total_space_bytes > 0 {
                stats.free_pad_space_bytes as f64 / stats.total_space_bytes as f64 * 100.0
            } else {
                0.0
            };
            let wasted_space_pct = if stats.total_space_bytes > 0 {
                stats.wasted_space_bytes as f64 / stats.total_space_bytes as f64 * 100.0
            } else {
                0.0
            };

            println!("Pad Usage Breakdown:");
            let gauge_width = 30;
            let (occ_gauge, occ_text, occ_color) =
                create_text_gauge(occupied_pads_pct, gauge_width, Color::LightRed);
            let (free_gauge, free_text, free_color) =
                create_text_gauge(free_pads_pct, gauge_width, Color::LightGreen);

            println!(
                "  Occupied: [{}]  {} ({} pads)",
                occ_color.paint(occ_gauge),
                occ_text,
                stats.occupied_pads
            );
            println!(
                "  Free:     [{}]  {} ({} pads)",
                free_color.paint(free_gauge),
                free_text,
                stats.free_pads
            );

            if stats.pending_verification_pads > 0 {
                println!(
                    "\n{}",
                    Color::Yellow.paint(format!(
                        "Note: {} pads are pending verification. Run 'mutant purge' to process them.",
                        stats.pending_verification_pads
                    ))
                );
            }

            println!("\nSpace Usage Breakdown (based on Total Space):");
            let (used_gauge, used_text, used_color) =
                create_text_gauge(used_space_pct, gauge_width, Color::LightCyan);
            let (wasted_gauge, wasted_text, wasted_color) =
                create_text_gauge(wasted_space_pct, gauge_width, Color::LightYellow);
            let (fspace_gauge, fspace_text, fspace_color) =
                create_text_gauge(free_space_pct, gauge_width, Color::LightGreen);

            println!(
                "  Used Data:[{}]  {} ({})",
                used_color.paint(used_gauge),
                used_text,
                format_bytes(stats.occupied_data_bytes)
            );
            println!(
                "  Wasted:   [{}]  {} ({})",
                wasted_color.paint(wasted_gauge),
                wasted_text,
                format_bytes(stats.wasted_space_bytes)
            );
            println!(
                "  Free:     [{}]  {} ({})",
                fspace_color.paint(fspace_gauge),
                fspace_text,
                format_bytes(stats.free_pad_space_bytes)
            );

            if stats.incomplete_keys_count > 0 {
                println!("\nIncomplete Uploads:");
                println!("-------------------");
                println!("  Keys:                {}", stats.incomplete_keys_count);
                println!(
                    "  Total Data Size:     {}",
                    format_bytes(stats.incomplete_keys_data_bytes)
                );
                println!(
                    "  Pads Associated:     {}",
                    stats.incomplete_keys_total_pads
                );
                println!(
                    "    Confirmed:         {}",
                    stats.incomplete_keys_pads_confirmed
                );
                println!(
                    "    Written (Pending): {}",
                    stats.incomplete_keys_pads_written
                );
                println!(
                    "    Generated (Pending): {}",
                    stats.incomplete_keys_pads_generated
                );
            }

            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error fetching stats: {}", e);
            ExitCode::FAILURE
        }
    }
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
