use crate::callbacks::progress::get_default_steps_style;
use clap::Args;
use indicatif::{MultiProgress, ProgressBar};
use log::{error, info};
// Use new top-level re-exports
use mutant_lib::{Error as LibError, MutAnt, PurgeCallback, PurgeEvent};
use std::error::Error as StdError; // Alias std::error::Error to avoid name clash
use std::sync::{Arc, Mutex as StdMutex}; // Use StdMutex for state that doesn't cross .await

#[derive(Args, Debug)]
pub struct PurgeArgs {}

// Shared state for the callback
#[derive(Clone, Default)]
struct PurgeState {
    // Wrap ProgressBar in StdMutex since indicatif::ProgressBar is !Sync
    pb: Option<Arc<StdMutex<ProgressBar>>>,
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
    let state = Arc::new(StdMutex::new(PurgeState::default()));

    // Create the callback closure, capturing quiet flag
    let mp_clone = mp.clone(); // Clone MultiProgress for the callback
    let state_clone = state.clone();
    let callback: PurgeCallback = Box::new(move |event: PurgeEvent| {
        let state_arc = state_clone.clone();
        let mp_clone = mp_clone.clone();
        let quiet_captured = quiet;

        // Return a Pinned, Boxed Future that is Send + Sync
        Box::pin(async move {
            // Lock the StdMutex - this does not require .await
            let mut state_guard = match state_arc.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    // Handle the poisoned mutex error, e.g., log and return an error
                    error!("Purge callback state mutex poisoned: {}", poisoned);
                    // Decide how to handle this - maybe return an internal error?
                    return Err(LibError::Internal(
                        "Purge callback state mutex poisoned".to_string(),
                    ));
                }
            };

            match event {
                PurgeEvent::Starting { total_count } => {
                    if !quiet_captured && total_count > 0 {
                        let pb = Arc::new(StdMutex::new(
                            mp_clone.add(ProgressBar::new(total_count as u64)),
                        ));
                        {
                            let pb_guard = pb.lock().unwrap(); // Lock the inner ProgressBar
                            pb_guard.set_style(get_default_steps_style());
                            pb_guard.set_message("Verifying pads...");
                        } // Inner lock guard dropped
                        state_guard.pb = Some(pb);
                    } else if total_count == 0 && quiet_captured {
                        println!("No pending pads found to purge.");
                    }
                    Ok::<bool, LibError>(true)
                }
                PurgeEvent::PadProcessed => {
                    if let Some(pb_arc) = &state_guard.pb {
                        // Lock inner PB to increment
                        match pb_arc.lock() {
                            Ok(pb) => pb.inc(1),
                            Err(poisoned) => {
                                error!(
                                    "Inner progress bar mutex poisoned in PadProcessed: {}",
                                    poisoned
                                );
                                // Ignore or return error?
                            }
                        }
                    }
                    Ok::<bool, LibError>(true)
                }
                PurgeEvent::Complete {
                    verified_count,
                    failed_count,
                } => {
                    let maybe_pb_arc = state_guard.pb.take(); // Take ownership from state
                    // Drop the state guard *before* potentially finishing the PB
                    drop(state_guard);

                    if let Some(pb_arc) = maybe_pb_arc {
                        // Lock inner PB
                        match pb_arc.lock() {
                            Ok(pb) => {
                                pb.finish_with_message(format!(
                                    "Purge complete. Verified: {}, Discarded: {}",
                                    verified_count, failed_count
                                ));
                            }
                            Err(poisoned) => {
                                error!(
                                    "Inner progress bar mutex poisoned in Complete: {}",
                                    poisoned
                                );
                            }
                        }
                    } else if !quiet_captured {
                        println!("No pending pads found to purge.");
                    }

                    if !quiet_captured {
                        info!(
                            "Purge summary: Verified: {}, Discarded: {}",
                            verified_count, failed_count
                        );
                    }
                    Ok::<bool, LibError>(true)
                }
            }
            // state_guard is implicitly dropped if not already
        })
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
