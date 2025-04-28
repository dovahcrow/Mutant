use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, warn};
use mutant_client::ProgressReceiver;
use mutant_protocol::{HealthCheckCallback, HealthCheckEvent, TaskProgress};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn create_health_check_progress(
    mut progress_rx: ProgressReceiver,
    multi_progress: &MultiProgress,
) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: HealthCheckCallback = Arc::new(move |event: HealthCheckEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        Box::pin(async move {
            match event {
                HealthCheckEvent::Starting { total_keys } => {
                    let mut pb_guard = pb_arc.lock().await;
                    let _ = pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&multi_progress);
                        pb.set_message("Health check: Processing keys...".to_string());
                        pb.set_length(total_keys as u64);
                        pb.set_position(0);
                        pb
                    });

                    drop(pb_guard);
                }
                HealthCheckEvent::KeyProcessed => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Health Check Callback: KeyProcessed event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
                HealthCheckEvent::Complete { nb_keys_updated: _ } => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                        }
                    } else {
                        error!(
                            "Health Check Callback: Complete event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
            }
            Ok(true)
        })
    });

    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            match progress {
                Ok(TaskProgress::HealthCheck(event)) => {
                    callback(event.clone()).await.unwrap();
                }
                Ok(_) => warn!("Unexpected progress type"),
                Err(e) => error!("Progress error: {:?}", e),
            }
        }
    });
}
