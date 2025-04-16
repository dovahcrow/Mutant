use crate::app::CliError;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info, trace, warn};
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::{Duration, interval};

#[derive(Args, Debug, Clone)]
pub struct Reserve {
    /// Number of scratchpads to reserve [default: 1]
    #[arg(value_name = "COUNT")]
    count: Option<usize>,
}

impl Reserve {
    pub async fn run(&self, mutant: &mutant_lib::api::MutAnt) -> Result<(), CliError> {
        // Default to 1 if count is not provided
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
        pb.set_message("Reserving pads");

        let mut join_set = JoinSet::new();

        // Clone mutant instance for use in tasks
        let mutant = Arc::new(mutant.clone());

        for i in 0..count {
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

        // Add a ticker interval
        let mut ticker = interval(Duration::from_millis(100)); // Tick every 100ms

        // Loop while there are tasks running using select!
        while !join_set.is_empty() {
            tokio::select! {
                // Prioritize checking for completed tasks first
                // biased; // Optional, uncomment if needed
                Some(result) = join_set.join_next() => {
                    pb.inc(1); // Increment when a task completes
                    match result {
                        Ok(Ok(address)) => {
                            successful_creations += 1;
                            pb.set_message(format!("Reserved {}", address));
                            trace!("Successfully processed reservation for {}", address);
                            // Save index immediately after successful reservation
                            if let Err(e) = mutant.save_index_cache().await {
                                // Log error but don't stop the whole process
                                error!(
                                    "Failed to save index cache after reserving {}: {}. Continuing...",
                                    address, e
                                );
                            }
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
                // If no task completed, check if the ticker interval elapsed
                _ = ticker.tick() => {
                     // Only tick the spinner if the bar hasn't finished yet
                     if !pb.is_finished() {
                        pb.tick();
                     }
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

        info!(
            "Successfully reserved {} scratchpads and added them to the free list (local cache only).",
            successful_creations
        );

        if failed_creations > 0 {
            warn!("{} pads failed to be reserved.", failed_creations);
        }

        Ok(())
    }
}
