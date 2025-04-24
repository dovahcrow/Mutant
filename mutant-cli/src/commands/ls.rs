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
                    " {:<15} {:>3} pads {:>10} {:>10}",
                    key,
                    pads.len(),
                    format_size(pads.iter().map(|p| p.size).sum::<usize>(), BINARY),
                    completion_str,
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
                    " {:<15} {:>3} pads {:>10} Public: {:>10} {}",
                    key,
                    pads.len() + 1,
                    format_size(
                        pads.iter().map(|p| p.size).sum::<usize>() + index_pad.size,
                        BINARY
                    ),
                    index_pad.address,
                    completion_str,
                );
            }
        }
    }

    info!("Loading fetch history from file...");
    let mut history = load_history();
    if !history.is_empty() {
        println!("\n--- Fetch History ---");
        history.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at));

        for entry in history {
            let size_str = format_size(entry.size, BINARY);
            let date_str = entry.fetched_at.format("%b %d %H:%M").to_string();

            println!(
                " {:<15} {:>8} {:>5} Public: {}",
                date_str, "", size_str, entry.address
            );
        }
    }
    ExitCode::SUCCESS
}
