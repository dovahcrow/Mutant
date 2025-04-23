// use crate::callbacks::StyledProgressBar;
// use crate::callbacks::get::create_get_callback;
use crate::history::{FetchHistoryEntry, append_history_entry};
use chrono::Utc;
use indicatif::MultiProgress;
use log::{debug, info};
use mutant_lib::MutAnt;
use mutant_lib::error::Error as LibError;
use mutant_lib::storage::ScratchpadAddress;
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn handle_get(
    mutant: MutAnt,
    key_or_address: String,
    public: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> ExitCode {
    debug!(
        "CLI: Handling Get command: key_or_address={}, public={}",
        key_or_address, public
    );

    // let (download_pb_opt, callback) = create_get_callback(multi_progress, quiet);

    let result: Result<Vec<u8>, LibError> = if public {
        match mutant
            .get_public(&ScratchpadAddress::from_hex(&key_or_address).unwrap())
            .await
        {
            Ok(data) => Ok(data),
            Err(e) => Err(e),
        }
        // debug!(
        //     "CLI: Fetching public data using address '{}'",
        //     key_or_address
        // );
        // match ScratchpadAddress::from_hex(&key_or_address) {
        //     Ok(address) => {
        //         let fetch_result = mutant.fetch_public(address, Some(callback)).await;

        //         if let Ok(data) = &fetch_result {
        //             info!("Recording fetch history for {}", address);
        //             let history_entry = FetchHistoryEntry {
        //                 address,
        //                 size: data.len(),
        //                 fetched_at: Utc::now(),
        //             };
        //             append_history_entry(history_entry);
        //         }

        //         fetch_result.map(|bytes| bytes.to_vec())
        //     }
        //     Err(e) => {
        //         eprintln!(
        //             "Error: Invalid public address format '{}': {}",
        //             key_or_address, e
        //         );
        //         abandon_pb(
        //             &download_pb_opt,
        //             format!("Invalid public address format: {}", e),
        //         );
        //         return ExitCode::FAILURE;
        //     }
        // }
    } else {
        match mutant.get(&key_or_address).await {
            Ok(data) => {
                // print each byte as it is
                io::stdout().write_all(&data).unwrap();
                Ok(data)
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
        // // Try fetching by name (could be private key or public upload name)
        // debug!(
        //     "CLI: Attempting to fetch by name '{}' (checking index first)...",
        //     key_or_address
        // );
        // match mutant.get_key_details(&key_or_address).await {
        //     Ok(Some(details)) => {
        //         if let Some(public_address) = details.public_address {
        //             // It's a public upload known by name
        //             debug!(
        //                 "CLI: Name '{}' is a public upload with address {}. Fetching publicly...",
        //                 key_or_address, public_address
        //             );
        //             mutant
        //                 .fetch_public(public_address, Some(callback))
        //                 .await
        //                 .map(|bytes| bytes.to_vec())
        //         } else {
        //             // It's a private key
        //             debug!(
        //                 "CLI: Name '{}' is a private key. Fetching privately...",
        //                 key_or_address
        //             );
        //             mutant
        //                 .fetch_with_progress(&key_or_address, Some(callback))
        //                 .await
        //         }
        //     }
        //     Ok(None) => {
        //         // Name not found in index (neither private nor public)
        //         Err(LibError::Data(DataError::KeyNotFound(
        //             key_or_address.clone(),
        //         )))
        //     }
        //     Err(e) => {
        //         // Error fetching details
        //         Err(e)
        //     }
        // }
    };

    ExitCode::SUCCESS

    // match result {
    //     Ok(value_bytes) => {
    //         clear_pb(&download_pb_opt);
    //         if let Err(e) = io::stdout().write_all(&value_bytes) {
    //             eprintln!("Error writing fetched data to stdout: {}", e);
    //             ExitCode::FAILURE
    //         } else {
    //             ExitCode::SUCCESS
    //         }
    //     }
    //     Err(e) => {
    //         let identifier = if public { "address" } else { "key" };
    //         let error_message = match e {
    //             LibError::Data(DataError::KeyNotFound(k)) => {
    //                 format!("Error: {} '{}' not found.", identifier, k)
    //             }
    //             LibError::Data(DataError::InternalError(msg)) if msg.contains("incomplete") => {
    //                 format!("Cannot fetch {}: {}", key_or_address, msg)
    //             }
    //             _ => format!(
    //                 "Error getting data for {} '{}': {}",
    //                 identifier, key_or_address, e
    //             ),
    //         };

    //         eprintln!("{}", error_message);
    //         abandon_pb(&download_pb_opt, error_message);

    //         ExitCode::FAILURE
    //     }
    // }
}

// fn clear_pb(pb_opt: &Arc<Mutex<Option<StyledProgressBar>>>) {
//     if let Ok(mut guard) = pb_opt.try_lock() {
//         if let Some(pb) = guard.take() {
//             if !pb.is_finished() {
//                 pb.finish_and_clear();
//             }
//         }
//     } else {
//         log::warn!("clear_pb: Could not acquire lock to clear progress bar.");
//     }
// }

// fn abandon_pb(pb_opt: &Arc<Mutex<Option<StyledProgressBar>>>, message: String) {
//     if let Ok(mut guard) = pb_opt.try_lock() {
//         if let Some(pb) = guard.take() {
//             if !pb.is_finished() {
//                 pb.abandon_with_message(message);
//             }
//         }
//     } else {
//         log::warn!("abandon_pb: Could not acquire lock to abandon progress bar.");
//     }
// }
