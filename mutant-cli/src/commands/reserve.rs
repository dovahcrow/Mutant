use crate::app::CliError;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info, trace, warn};
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Args, Debug, Clone)]
pub struct Reserve {
    /// Number of scratchpads to reserve
    #[arg(short, long, value_name = "COUNT", default_value_t = 1)]
    count: usize,
}

impl Reserve {
    pub async fn run(&self, mutant: &mutant_lib::api::MutAnt) -> Result<(), CliError> {
        if self.count == 0 {
            info!("Reserve count is 0, nothing to do.");
            return Ok(());
        }

        info!("Attempting to reserve {} new scratchpads...", self.count);

        let pb = ProgressBar::new(self.count as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta}) {msg}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        pb.set_message("Reserving pads");

        let mut join_set = JoinSet::new();

        // Clone mutant instance for use in tasks
        let mutant = Arc::new(mutant.clone());

        for i in 0..self.count {
            let mutant_clone = Arc::clone(&mutant);
            join_set.spawn(async move {
                trace!("Task {}: Calling reserve_new_pad", i);
                match mutant_clone.reserve_new_pad().await {
                    Ok(created_address) => {
                        trace!(
                            "Task {}: Successfully reserved pad on network and added to index: {}",
                            i, created_address
                        );
                        Ok(created_address)
                    }
                    Err(e) => {
                        error!("Task {}: Failed to reserve pad: {}", i, e);
                        Err(e)
                    }
                }
            });
        }

        let mut successful_creations = 0;
        let mut failed_creations = 0;

        while let Some(result) = join_set.join_next().await {
            pb.inc(1);
            match result {
                Ok(Ok(address)) => {
                    successful_creations += 1;
                    pb.set_message(format!("Reserved {}", address));
                    trace!("Successfully processed reservation for {}", address);
                }
                Ok(Err(lib_err)) => {
                    failed_creations += 1;
                    pb.set_message(format!("Failed: {}", lib_err));
                    warn!("A pad reservation task failed: {}", lib_err);
                }
                Err(join_err) => {
                    failed_creations += 1;
                    pb.set_message(format!("Task failed: {}", join_err));
                    error!("Pad reservation join error: {}", join_err);
                }
            }
        }

        pb.finish_with_message(format!(
            "Reservation complete: {} succeeded, {} failed",
            successful_creations, failed_creations
        ));

        if successful_creations == 0 {
            error!("No scratchpads were successfully reserved.");
            return Err(CliError::MutAntInit(
                "Failed to reserve any scratchpads".to_string(),
            ));
        }

        // Save the index only once after all successful reservations
        info!(
            "Saving master index after reserving {} pads...",
            successful_creations
        );
        mutant.save_master_index().await?;
        info!("Master index saved successfully.");

        info!(
            "Successfully reserved {} scratchpads and added them to the free list.",
            successful_creations
        );

        if failed_creations > 0 {
            warn!("{} pads failed to be reserved.", failed_creations);
        }

        Ok(())
    }
}
