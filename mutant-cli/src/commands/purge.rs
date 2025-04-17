use crate::callbacks::progress::get_default_steps_style;
use clap::Args;
use indicatif::{MultiProgress, ProgressBar};
use log::{error, info};

use mutant_lib::{Error as LibError, MutAnt, PurgeCallback, PurgeEvent};
use std::error::Error as StdError;
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Args, Debug)]
pub struct PurgeArgs {}

#[derive(Clone, Default)]
struct PurgeState {
    pb: Option<Arc<StdMutex<ProgressBar>>>,
}

pub async fn run(
    _args: PurgeArgs,
    mutant: MutAnt,
    mp: &MultiProgress,
    quiet: bool,
) -> Result<(), Box<dyn StdError>> {
    info!("Executing purge command...");

    let state = Arc::new(StdMutex::new(PurgeState::default()));

    let mp_clone = mp.clone();
    let state_clone = state.clone();
    let callback: PurgeCallback = Box::new(move |event: PurgeEvent| {
        let state_arc = state_clone.clone();
        let mp_clone = mp_clone.clone();
        let quiet_captured = quiet;

        Box::pin(async move {
            let mut state_guard = match state_arc.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    error!("Purge callback state mutex poisoned: {}", poisoned);

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
                            let pb_guard = pb.lock().unwrap();
                            pb_guard.set_style(get_default_steps_style());
                            pb_guard.set_message("Verifying pads...");
                        }
                        state_guard.pb = Some(pb);
                    } else if total_count == 0 && quiet_captured {
                        println!("No pending pads found to purge.");
                    }
                    Ok::<bool, LibError>(true)
                }
                PurgeEvent::PadProcessed => {
                    if let Some(pb_arc) = &state_guard.pb {
                        match pb_arc.lock() {
                            Ok(pb) => pb.inc(1),
                            Err(poisoned) => {
                                error!(
                                    "Inner progress bar mutex poisoned in PadProcessed: {}",
                                    poisoned
                                );
                            }
                        }
                    }
                    Ok::<bool, LibError>(true)
                }
                PurgeEvent::Complete {
                    verified_count,
                    failed_count,
                } => {
                    let maybe_pb_arc = state_guard.pb.take();

                    drop(state_guard);

                    if let Some(pb_arc) = maybe_pb_arc {
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
        })
    });

    match mutant.purge(Some(callback)).await {
        Ok(_) => Ok(()),
        Err(e) => {
            if matches!(e, LibError::OperationCancelled) {
                info!("Purge operation cancelled.");
                Ok(())
            } else {
                Err(Box::new(e) as Box<dyn StdError>)
            }
        }
    }
}
