use crate::callbacks::get::create_get_callback;
use crate::history::{FetchHistoryEntry, append_history_entry};
use chrono::Utc;
use indicatif::MultiProgress;
use log::{debug, info};
use mutant_lib::MutAnt;
use mutant_lib::storage::ScratchpadAddress;
use std::io::{self, Write};
use std::process::ExitCode;
pub async fn handle_get(
    mut mutant: MutAnt,
    key_or_address: String,
    public: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!(
        "CLI: Handling Get command: key_or_address={}, public={}",
        key_or_address, public
    );

    let (_download_pb_opt, callback) = create_get_callback(multi_progress, quiet);

    mutant.set_get_callback(callback).await;

    if public {
        match mutant
            .get_public(&ScratchpadAddress::from_hex(&key_or_address).unwrap())
            .await
        {
            Ok(data) => {
                info!("Recording fetch history for {}", key_or_address);
                let history_entry = FetchHistoryEntry {
                    address: ScratchpadAddress::from_hex(&key_or_address).unwrap(),
                    size: data.len(),
                    fetched_at: Utc::now(),
                };
                append_history_entry(history_entry);

                // print each byte as it is
                io::stdout().write_all(&data).unwrap();
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        match mutant.get(&key_or_address).await {
            Ok(data) => {
                // print each byte as it is
                io::stdout().write_all(&data).unwrap();
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
    }
}
