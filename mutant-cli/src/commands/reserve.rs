use crate::app::CliError;
use crate::callbacks::progress::StyledProgressBar;
use clap::Args;
use indicatif::MultiProgress;
use log::{error, info};
use mutant_lib::MutAnt;
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{ReserveCallback, ReserveEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Args, Debug, Clone)]
pub struct Reserve {
    #[arg(value_name = "COUNT")]
    count: Option<usize>,
}

#[derive(Clone)]
struct ReserveCallbackContext {
    pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    multi_progress: MultiProgress,
}

impl Reserve {
    pub async fn run(
        &self,
        mutant: &MutAnt,
        multi_progress: &MultiProgress,
    ) -> Result<(), CliError> {
        let count = self.count.unwrap_or(1);
        if count == 0 {
            info!("Reserve count is 0, nothing to do.");
            return Ok(());
        }

        info!("Attempting to reserve {} new scratchpads...", count);

        let pb_opt_arc = Arc::new(Mutex::new(None::<StyledProgressBar>));

        let context = ReserveCallbackContext {
            pb_opt: pb_opt_arc.clone(),
            multi_progress: multi_progress.clone(),
        };

        let callback: ReserveCallback = Box::new(move |event: ReserveEvent| {
            let ctx = context.clone();
            Box::pin(async move {
                let mut pb_guard = ctx.pb_opt.lock().await;

                match event {
                    ReserveEvent::Starting { total_requested } => {
                        let pb = pb_guard.get_or_insert_with(|| {
                            let spb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                            spb.set_message("Initializing...".to_string());
                            spb
                        });
                        pb.set_length(total_requested as u64);
                        pb.set_position(0);
                        pb.set_message("Reserving pads...".to_string());
                    }
                    ReserveEvent::PadReserved { address } => {
                        if let Some(pb) = pb_guard.as_mut() {
                            if !pb.is_finished() {
                                pb.inc(1);
                                pb.set_message(format!("Reserved {}", address));
                            }
                        }
                    }
                    ReserveEvent::SavingIndex { reserved_count: _ } => {
                        if let Some(pb) = pb_guard.as_mut() {
                            if !pb.is_finished() {
                                pb.set_message("Saving index...".to_string());
                            }
                        }
                    }
                    ReserveEvent::Complete { succeeded, failed } => {
                        if let Some(pb) = pb_guard.take() {
                            pb.finish_with_message(format!(
                                "Reservation complete: {} succeeded, {} failed",
                                succeeded, failed
                            ));
                        }
                    }
                }
                drop(pb_guard);
                Ok::<bool, LibError>(true)
            })
        });

        match mutant.reserve_pads(count, Some(callback)).await {
            Ok(successful_creations) => {
                info!(
                    "Successfully reserved {} scratchpads (library call complete).",
                    successful_creations
                );
                if successful_creations == 0 {
                    error!(
                        "No scratchpads were successfully reserved despite library call succeeding."
                    );
                    let mut pb_guard = pb_opt_arc.lock().await;
                    if let Some(pb) = pb_guard.take() {
                        pb.finish_with_message("Reservation complete: 0 succeeded");
                    }
                    return Err(CliError::MutAntInit(
                        "Failed to reserve any scratchpads".to_string(),
                    ));
                }
                Ok(())
            }
            Err(e) => {
                error!("Pad reservation failed: {}", e);
                let mut pb_guard = pb_opt_arc.lock().await;
                if let Some(pb) = pb_guard.take() {
                    pb.finish_with_message(format!("Reservation failed: {}", e));
                }
                Err(CliError::MutAntInit(format!(
                    "Pad reservation failed: {}",
                    e
                )))
            }
        }
    }
}
