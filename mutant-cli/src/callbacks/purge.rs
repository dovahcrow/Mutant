use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, trace, warn};
use mutant_client::ProgressReceiver;
use mutant_protocol::{PurgeCallback, PurgeEvent, TaskProgress};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn create_purge_progress(mut progress_rx: ProgressReceiver, multi_progress: &MultiProgress) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: PurgeCallback = Arc::new(move |event: PurgeEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        Box::pin(async move {
            match event {
                PurgeEvent::Starting { total_count } => {
                    let mut pb_guard = pb_arc.lock().await;
                    let _ = pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&multi_progress);
                        pb.set_message("Purging pads...".to_string());
                        pb.set_length(total_count as u64);
                        pb.set_position(0);
                        pb
                    });

                    drop(pb_guard);
                }
                PurgeEvent::PadProcessed => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Purge Callback: PadProcessed event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
                PurgeEvent::Complete {
                    verified_count,
                    failed_count,
                } => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.take() {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                            trace!("Purge Callback: Complete - Progress bar finished and cleared.");
                        }
                    } else {
                        trace!(
                            "Purge Callback: Complete event received but progress bar was already finished or never existed."
                        );
                    }
                    drop(pb_guard);

                    println!(
                        "Purge complete. Verified: {}, Discarded: {}",
                        verified_count, failed_count
                    );
                }
            }
            Ok(true)
        })
    });

    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            match progress {
                Ok(TaskProgress::Purge(event)) => {
                    callback(event.clone()).await.unwrap();
                }
                Ok(_) => warn!("Unexpected progress type"),
                Err(e) => error!("Progress error: {:?}", e),
            }
        }
    });
}
