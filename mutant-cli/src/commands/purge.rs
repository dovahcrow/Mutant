use crate::callbacks::progress::get_default_steps_style;
use clap::Args;
use indicatif::{MultiProgress, ProgressBar};
use log::info;
use mutant_lib::MutAnt;
use std::error::Error;

#[derive(Args, Debug)]
pub struct PurgeArgs {}

// Accept MutAnt and MultiProgress instance, return standard Result
pub async fn run(
    _args: PurgeArgs,
    mutant: MutAnt,
    mp: &MultiProgress,
) -> Result<(), Box<dyn Error>> {
    info!("Executing purge command...");

    // Call the library function first to get the total count
    let (initial_count, verified_count, failed_count) = mutant
        .purge_unverified_pads()
        .await
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;

    // Create progress bar only if there were pads to process
    if initial_count > 0 {
        let pb = mp.add(ProgressBar::new(initial_count as u64));
        pb.set_style(get_default_steps_style());
        pb.set_message(format!(
            "Purged pads. Verified: {}, Discarded: {}",
            verified_count, failed_count
        ));
        // Set the final position to the total number processed
        pb.set_position(initial_count as u64);
        pb.finish(); // Mark as finished, keep the message
    } else {
        // Optionally print a message if nothing to do
        println!("No pending pads found to purge.");
    }

    Ok(())
}
