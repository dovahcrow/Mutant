use crate::app::CliError;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info};
use mutant_lib::api::{ReserveCallback, ReserveEvent};
use mutant_lib::error::Error as LibError;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Args, Debug, Clone)]
pub struct Reserve {
    #[arg(value_name = "COUNT")]
    count: Option<usize>,
}

#[derive(Clone)]
struct ReserveCallbackContext {
    pb: Arc<Mutex<ProgressBar>>,
}

impl Reserve {
    pub async fn run(&self, mutant: &mutant_lib::api::MutAnt) -> Result<(), CliError> {
        let count = self.count.unwrap_or(1);
        if count == 0 {
            info!("Reserve count is 0, nothing to do.");
            return Ok(());
        }

        info!("Attempting to reserve {} new scratchpads...", count);

        let pb = ProgressBar::new(count as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta}) {msg}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        pb.set_message("Initializing...");
        let pb_arc = Arc::new(Mutex::new(pb));

        let context = ReserveCallbackContext { pb: pb_arc.clone() };

        let callback: ReserveCallback = Box::new(move |event: ReserveEvent| {
            let ctx = context.clone();
            Box::pin(async move {
                let pb_guard = ctx.pb.lock().await;
                match event {
                    ReserveEvent::Starting { total_requested } => {
                        pb_guard.set_length(total_requested as u64);
                        pb_guard.set_position(0);
                        pb_guard.set_message("Reserving pads...");
                    }
                    ReserveEvent::PadReserved { address } => {
                        pb_guard.inc(1);
                        pb_guard.set_message(format!("Reserved {}", address));
                    }
                    ReserveEvent::SavingIndex { reserved_count: _ } => {
                        pb_guard.set_message("Saving index...");
                        pb_guard.tick();
                    }
                    ReserveEvent::Complete { succeeded, failed } => {
                        pb_guard.finish_with_message(format!(
                            "Reservation complete: {} succeeded, {} failed",
                            succeeded, failed
                        ));
                    }
                }

                if !pb_guard.is_finished() {
                    pb_guard.tick();
                }
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

                    let pb_guard = pb_arc.lock().await;
                    if !pb_guard.is_finished() {
                        pb_guard.finish_with_message("Reservation complete: 0 succeeded");
                    }
                    return Err(CliError::MutAntInit(
                        "Failed to reserve any scratchpads".to_string(),
                    ));
                }

                Ok(())
            }
            Err(e) => {
                error!("Pad reservation failed: {}", e);

                let pb_guard = pb_arc.lock().await;
                if !pb_guard.is_finished() {
                    pb_guard.finish_with_message(format!("Reservation failed: {}", e));
                }
                Err(CliError::MutAntInit(format!(
                    "Pad reservation failed: {}",
                    e
                )))
            }
        }
    }
}
