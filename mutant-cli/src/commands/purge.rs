use crate::callbacks::progress::get_default_steps_style;
use clap::Args;
use futures::future::FutureExt;
use indicatif::{MultiProgress, ProgressBar};
use log::info;
// Use new top-level re-exports
use mutant_lib::{Error as LibError, MutAnt, PurgeCallback, PurgeEvent};
use std::error::Error as StdError; // Alias std::error::Error to avoid name clash
use std::sync::{Arc, Mutex};

#[derive(Args, Debug)]
pub struct PurgeArgs {}

// Shared state for the callback
#[derive(Clone, Default)]
struct PurgeState {
    pb: Option<Arc<ProgressBar>>,
}

pub async fn run(
    _args: PurgeArgs,
    mutant: MutAnt,
    mp: &MultiProgress,
    quiet: bool, // Add quiet flag
) -> Result<(), Box<dyn StdError>> {
    // Use aliased StdError
    info!("Executing purge command...");

    // Shared state between callback instances
    let state = Arc::new(Mutex::new(PurgeState::default()));

    // Create the callback closure, capturing quiet flag
    let mp_clone = mp.clone(); // Clone MultiProgress for the callback
    let state_clone = state.clone();
    let callback: PurgeCallback = Box::new(move |event: PurgeEvent| {
        let state = state_clone.clone();
        let mp = mp_clone.clone();
        let quiet_captured = quiet; // Capture quiet flag
        async move {
            let mut state_guard = state.lock().unwrap(); // Use std::sync::Mutex, so unwrap is safe
            match event {
                PurgeEvent::Starting { total_count } => {
                    // Only create progress bar if not quiet
                    if !quiet_captured && total_count > 0 {
                        let pb = Arc::new(mp.add(ProgressBar::new(total_count as u64)));
                        pb.set_style(get_default_steps_style());
                        pb.set_message("Verifying pads...");
                        state_guard.pb = Some(pb);
                    } else if total_count == 0 {
                        // Optionally print message if quiet and 0 pads
                        if quiet_captured {
                            println!("No pending pads found to purge.");
                        }
                    }
                }
                PurgeEvent::PadProcessed => {
                    // Only increment if pb exists
                    if let Some(pb) = &state_guard.pb {
                        pb.inc(1);
                    }
                }
                PurgeEvent::Complete {
                    verified_count,
                    failed_count,
                } => {
                    // Only finish/print if pb exists
                    if let Some(pb) = state_guard.pb.take() {
                        pb.finish_with_message(format!(
                            "Purge complete. Verified: {}, Discarded: {}",
                            verified_count, failed_count
                        ));
                    } else if !quiet_captured {
                        // If not quiet and pb was None (meaning 0 pads initially), print the message.
                        println!("No pending pads found to purge.");
                    }
                    // Always print summary if not quiet, or maybe always? Decide based on desired quiet behavior.
                    // For now, let's assume quiet suppresses this too.
                    if !quiet_captured {
                        info!(
                            "Purge summary: Verified: {}, Discarded: {}",
                            verified_count, failed_count
                        );
                    }
                }
            }
            Ok(true) // Indicate to continue processing
        }
        .boxed()
    });

    // Call the library function with the callback
    // Call the library function (now named purge) with the callback
    match mutant.purge(Some(callback)).await {
        Ok(_) => Ok(()),
        Err(e) => {
            // Handle potential OperationCancelled error from the callback
            if matches!(e, LibError::OperationCancelled) {
                // Use LibError::OperationCancelled
                info!("Purge operation cancelled.");
                Ok(()) // Return Ok(()) if cancelled
            } else {
                // Map other LibError types to Box<dyn StdError>
                Err(Box::new(e) as Box<dyn StdError>)
            }
        }
    }
}
