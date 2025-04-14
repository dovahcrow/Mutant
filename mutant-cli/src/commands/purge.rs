use crate::callbacks::progress::get_default_steps_style;
use clap::Args;
use futures::future::FutureExt;
use indicatif::{MultiProgress, ProgressBar};
use log::info;
use mutant_lib::MutAnt;
use mutant_lib::events::{PurgeCallback, PurgeEvent};
use std::error::Error;
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
) -> Result<(), Box<dyn Error>> {
    info!("Executing purge command...");

    // Shared state between callback instances
    let state = Arc::new(Mutex::new(PurgeState::default()));

    // Create the callback closure
    let mp_clone = mp.clone(); // Clone MultiProgress for the callback
    let state_clone = state.clone();
    let callback: PurgeCallback = Box::new(move |event: PurgeEvent| {
        let state = state_clone.clone();
        let mp = mp_clone.clone();
        async move {
            let mut state_guard = state.lock().unwrap(); // Use std::sync::Mutex, so unwrap is safe
            match event {
                PurgeEvent::Starting { total_count } => {
                    if total_count > 0 {
                        let pb = Arc::new(mp.add(ProgressBar::new(total_count as u64)));
                        pb.set_style(get_default_steps_style());
                        pb.set_message("Verifying pads...");
                        state_guard.pb = Some(pb);
                    } else {
                        // No pads to process, print message and finish early?
                        // Or let it run through and Complete will handle it.
                    }
                }
                PurgeEvent::PadProcessed => {
                    if let Some(pb) = &state_guard.pb {
                        pb.inc(1);
                    }
                }
                PurgeEvent::Complete {
                    verified_count,
                    failed_count,
                } => {
                    if let Some(pb) = state_guard.pb.take() {
                        pb.finish_with_message(format!(
                            "Purge complete. Verified: {}, Discarded: {}",
                            verified_count, failed_count
                        ));
                    } else {
                        // Handle case where there were 0 pads initially
                        println!("No pending pads found to purge.");
                    }
                }
            }
            Ok(true) // Indicate to continue processing
        }
        .boxed()
    });

    // Call the library function with the callback
    match mutant.purge_unverified_pads(Some(callback)).await {
        Ok(_) => Ok(()),
        Err(e) => {
            // Handle potential OperationCancelled error from the callback
            if matches!(e, mutant_lib::error::Error::OperationCancelled) {
                info!("Purge operation cancelled.");
                // Return success or a specific cancellation error if needed
                // For now, let's return success from CLI perspective if user cancelled.
                Ok(())
            } else {
                // Map other library errors to Box<dyn Error>
                Err(Box::new(e) as Box<dyn Error>)
            }
        }
    }
}
