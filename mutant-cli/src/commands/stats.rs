// use chrono::{DateTime, Utc};
use humansize::{BINARY, format_size};
use log::{debug, info};

use mutant_lib::MutAnt;
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
            println!("Total Pads Managed:    {}", stats.total_pads);
            println!("  Occupied (Private):  {}", stats.occupied_pads);
            println!("  Free Pads:           {}", stats.free_pads);
            println!("  Pending Verify Pads: {}", stats.pending_verification_pads);
            println!(
                "  Incomplete Alloc/Wrtn: {}",
                stats.incomplete_keys_pads_allocated_written
            );
            println!("  Occupied (Public Idx): {}", stats.public_data_pad_count); // Note: public_data_pad_count is index pads only

            println!("-------------------");
            println!(
                "Total Space Managed:   {}",
                format_bytes(stats.total_space_bytes)
            );
            println!(
                "  Occupied Pad Space (Private):  {}",
                format_bytes(stats.occupied_pad_space_bytes)
            );
            println!(
                "  Free Pad Space:      {}",
                format_bytes(stats.free_pad_space_bytes)
            );
            println!(
                "  Occupied Pad Space (Public Idx): {}",
                format_bytes(stats.public_data_space_bytes) // Note: public_data_space_bytes is index pads only
            );
            println!("-------------------");
            println!(
                "Actual Data Stored (Private):    {}",
                format_bytes(stats.occupied_data_bytes)
            );
            println!(
                "Actual Data Stored (Public):     {}",
                format_bytes(stats.public_data_actual_bytes)
            );
            println!(
                "Wasted Space (Private Frag.):  {}",
                format_bytes(stats.wasted_space_bytes)
            );
            println!(
                "Wasted Space (Public Idx): {}", // Clarify meaning
                format_bytes(stats.public_data_wasted_bytes)
            );
            println!("-------------------");

            // --- Pad Usage Gauge ---
            let total_pads_f64 = stats.total_pads as f64;
            let occupied_confirmed_pads = stats.occupied_pads + stats.public_data_pad_count; // Confirmed Private + Public Index
            let unavailable_pads = stats.free_pads
                + stats.pending_verification_pads
                + stats.incomplete_keys_pads_allocated_written;

            let occupied_confirmed_pct = if stats.total_pads > 0 {
                occupied_confirmed_pads as f64 / total_pads_f64 * 100.0
            } else {
                0.0
            };
            // Calculate remaining percentage for the second bar
            let unavailable_pct = if stats.total_pads > 0 {
                unavailable_pads as f64 / total_pads_f64 * 100.0
            } else {
                0.0
            };

            // Normalize percentages if sum > 100 due to rounding or potential logic edge cases
            let total_pct_pad = occupied_confirmed_pct + unavailable_pct;
            let (occupied_confirmed_pct, unavailable_pct) =
                if total_pct_pad > 100.0 && total_pct_pad < 101.0 {
                    // Allow small rounding errors
                    // Simple normalization: scale down proportionally
                    let scale = 100.0 / total_pct_pad;
                    (occupied_confirmed_pct * scale, unavailable_pct * scale)
                } else {
                    (occupied_confirmed_pct, unavailable_pct)
                };

            println!("Pad Usage Breakdown:");
            let gauge_width = 30;
            let (occ_gauge, occ_text, occ_color) =
                create_text_gauge(occupied_confirmed_pct, gauge_width, Color::LightRed);
            // Use a different color for unavailable to distinguish from simple 'free'
            let (unavail_gauge, unavail_text, unavail_color) =
                create_text_gauge(unavailable_pct, gauge_width, Color::DarkGray);

            println!(
                "  Occupied (Conf P + Pub I): [{}] {} ({} pads)",
                occ_color.paint(occ_gauge),
                occ_text,
                occupied_confirmed_pads
            );
            println!(
                "  Unavailable (F+P+Inc A/W): [{}] {} ({} pads)", // F=Free, P=Pending, Inc A/W=Incomplete Alloc/Written
                unavail_color.paint(unavail_gauge),
                unavail_text,
                unavailable_pads
            );

            // --- Space Usage Gauge ---
            let total_space_f64 = stats.total_space_bytes as f64;
            let total_used_data_bytes = stats.occupied_data_bytes + stats.public_data_actual_bytes;
            let total_wasted_overhead_bytes =
                stats.wasted_space_bytes + stats.public_data_wasted_bytes;
            let total_free_space_bytes = stats.free_pad_space_bytes; // Already calculated

            let used_data_pct = if stats.total_space_bytes > 0 {
                total_used_data_bytes as f64 / total_space_f64 * 100.0
            } else {
                0.0
            };
            let wasted_overhead_pct = if stats.total_space_bytes > 0 {
                total_wasted_overhead_bytes as f64 / total_space_f64 * 100.0
            } else {
                0.0
            };
            let free_space_pct = if stats.total_space_bytes > 0 {
                total_free_space_bytes as f64 / total_space_f64 * 100.0
            } else {
                0.0
            };

            // Normalize percentages if sum > 100 due to rounding or potential logic edge cases
            let total_pct_space = used_data_pct + wasted_overhead_pct + free_space_pct;
            let (used_data_pct, wasted_overhead_pct, free_space_pct) =
                if total_pct_space > 100.0 && total_pct_space < 101.0 {
                    // Allow small rounding errors
                    let scale = 100.0 / total_pct_space;
                    (
                        used_data_pct * scale,
                        wasted_overhead_pct * scale,
                        free_space_pct * scale,
                    )
                } else {
                    (used_data_pct, wasted_overhead_pct, free_space_pct)
                };

            println!("\nSpace Usage Breakdown (based on Total Space Managed):");
            // Create gauges - using 3 requires careful formatting
            let (used_gauge, used_text, used_color) =
                create_text_gauge(used_data_pct, gauge_width, Color::LightCyan);
            let (wasted_gauge, wasted_text, wasted_color) =
                create_text_gauge(wasted_overhead_pct, gauge_width, Color::LightYellow);
            let (fspace_gauge, fspace_text, fspace_color) =
                create_text_gauge(free_space_pct, gauge_width, Color::LightGreen);

            // Displaying 3 bars might look cluttered, consider alternatives if needed.
            // For now, let's print them separately.
            println!(
                "  Used Data (P+P): [{}] {} ({})", // P+P = Private + Public
                used_color.paint(used_gauge),
                used_text,
                format_bytes(total_used_data_bytes)
            );
            println!(
                "  Wasted/Ovhd(P+PI):[{}] {} ({})", // P = Private Frag, PI = Public Index Overhead
                wasted_color.paint(wasted_gauge),
                wasted_text,
                format_bytes(total_wasted_overhead_bytes)
            );
            println!(
                "  Free Pad Space:  [{}] {} ({})",
                fspace_color.paint(fspace_gauge),
                fspace_text,
                format_bytes(total_free_space_bytes)
            );

            // --- Incomplete/Public Sections (remain mostly the same, maybe minor label tweaks) ---
            if stats.incomplete_keys_count > 0 {
                println!("\nIncomplete Private Uploads:");
                println!("---------------------------");
                println!(
                    "  Keys:                     {}",
                    stats.incomplete_keys_count
                );
                println!(
                    "  Total Data Size Estimate: {}",
                    format_bytes(stats.incomplete_keys_data_bytes)
                );
                println!(
                    "  Pads Associated:          {}",
                    stats.incomplete_keys_total_pads
                );
                println!(
                    "    Confirmed:              {}",
                    stats.incomplete_keys_pads_confirmed
                );
                println!(
                    "    Allocated/Written:      {}",
                    stats.incomplete_keys_pads_allocated_written
                );
                println!(
                    "    Generated (Not Written):{}",
                    stats.incomplete_keys_pads_generated
                );
            }

            if stats.public_index_count > 0 {
                println!("\nPublic Upload Statistics:");
                println!("-------------------------");
                println!("  Public Names (Indices):  {}", stats.public_index_count);
                println!(
                    "    Index Pad Space:       {}", // Total potential space for indices
                    format_bytes(stats.public_index_space_bytes)
                );
                println!(
                    "  Total Public Data Size:  {}", // Actual data size
                    format_bytes(stats.public_data_actual_bytes)
                );
                println!("  Unique Index Pads Used:  {}", stats.public_data_pad_count);
                println!(
                    "    Index Pad Space Used:  {}", // Space consumed by the unique index pads
                    format_bytes(stats.public_data_space_bytes)
                );
                println!(
                    "    Wasted (Index Pad Space vs Data Size): {}", // Difference between index space and actual data
                    format_bytes(stats.public_data_wasted_bytes)
                );
                println!(
                    "    (Note: Public Pad stats count index pads only, not underlying data pads)"
                );
            }

            if stats.pending_verification_pads > 0 {
                println!(
                    "\n{}",
                    Color::Yellow.paint(format!(
                        "Note: {} pads are pending verification. Run 'mutant purge' to process them.",
                        stats.pending_verification_pads
                    ))
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
