use crate::history::load_history;
use humansize::{BINARY, format_size};
use log::{debug, info};
use mutant_lib::{
    MutAnt,
    storage::{IndexEntry, PadStatus},
};
use std::process::ExitCode;

pub async fn handle_ls(mutant: MutAnt) -> ExitCode {
    debug!("CLI: Handling Ls command");

    let index = mutant.list().await.unwrap();

    for (key, entry) in index {
        match entry {
            IndexEntry::PrivateKey(pads) => {
                let completion = pads
                    .iter()
                    .filter(|p| p.status == PadStatus::Confirmed)
                    .count();

                let completion_str = if completion == pads.len() {
                    "".to_string()
                } else {
                    format!(
                        " {}% complete ({}/{})",
                        completion * 100 / pads.len(),
                        completion,
                        pads.len()
                    )
                };

                println!(
                    " {:<10} {:>3} pads {:>5} {}",
                    key,
                    pads.len(),
                    format_size(pads.iter().map(|p| p.size).sum::<usize>(), BINARY),
                    completion_str
                );
            }
            IndexEntry::PublicUpload(index_pad, pads) => {
                let completion = pads
                    .iter()
                    .filter(|p| p.status == PadStatus::Confirmed)
                    .count()
                    + if index_pad.status == PadStatus::Confirmed {
                        1
                    } else {
                        0
                    };

                let completion_str = if completion == pads.len() + 1 {
                    "".to_string()
                } else {
                    format!(
                        " {}% complete ({}/{})",
                        completion * 100 / (pads.len() + 1),
                        completion,
                        pads.len() + 1
                    )
                };

                println!(
                    " {:<10} {:>3} pads {:>5} {} (public @ {})",
                    key,
                    pads.len() + 1,
                    format_size(
                        pads.iter().map(|p| p.size).sum::<usize>() + index_pad.size,
                        BINARY
                    ),
                    completion_str,
                    index_pad.address
                );
            }
        }

        let max_size_width = 8;

        info!("Loading fetch history from file...");
        let mut history = load_history();
        if !history.is_empty() {
            println!("\n--- Fetch History ---");
            history.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at));

            let max_hist_size_width = history
                .iter()
                .map(|h| format_size(h.size, BINARY).len())
                .max()
                .unwrap_or(max_size_width);
            let max_hist_type_width = 8;

            println!(
                "{:<width$} {:<type_width$} {:<12} ADDRESS",
                "SIZE",
                "TYPE",
                "FETCHED",
                width = max_hist_size_width,
                type_width = max_hist_type_width
            );
            for entry in history {
                let size_str = format_size(entry.size, BINARY);
                let date_str = entry.fetched_at.format("%b %d %H:%M").to_string();
                println!(
                    "{:<width$} {:<type_width$} {:<12} {}",
                    size_str,
                    "Fetched",
                    date_str,
                    entry.address,
                    width = max_hist_size_width,
                    type_width = max_hist_type_width
                );
            }
        }

        // info!("Fetching key/upload summary...");
        // match mutant.list_keys().await {
        //     Ok(summaries) => {
        //         if summaries.is_empty() {
        //             println!("No keys or public uploads stored.");
        //         } else {
        //             let mut summaries = summaries;
        //             summaries.sort_by(|a, b| a.name.cmp(&b.name));

        //             for summary in summaries {
        //                 if summary.is_public {
        //                     if let Some(addr) = summary.address {
        //                         println!("{} (public @ {})", summary.name, addr);
        //                     } else {
        //                         println!("{} (public, address missing)", summary.name);
        //                     }
        //                 } else {
        //                     match mutant.get_key_details(&summary.name).await {
        //                         Ok(Some(detail)) => {
        //                             if detail.is_finished {
        //                                 println!("{}", summary.name);
        //                             } else if let Some(percentage) = detail.completion_percentage {
        //                                 println!("*{} ({:.1}%)", summary.name, percentage);
        //                             } else {
        //                                 println!("*{}", summary.name);
        //                             }
        //                         }
        //                         Ok(None) => {
        //                             eprintln!(
        //                                 "Warning: Index inconsistency for key '{}'. Details not found.%",
        //                                 summary.name
        //                             );
        //                             println!("{}", summary.name);
        //                         }
        //                         Err(e) => {
        //                             eprintln!(
        //                                 "Warning: Could not fetch details for key '{}': {}",
        //                                 summary.name, e
        //                             );
        //                             println!("{}", summary.name);
        //                         }
        //                     }
        //                 }
        //             }
        //         }
        //         ExitCode::SUCCESS
        //     }
        //     Err(e) => {
        //         eprintln!("Error fetching key/upload summary: {}", e);
        //         ExitCode::FAILURE
        //     }
        // }
    }
    ExitCode::SUCCESS
}
