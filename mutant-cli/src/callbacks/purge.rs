use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, trace};
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{PurgeCallback, PurgeEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

// Get the specific styles needed
use super::progress::get_default_steps_style;

pub fn create_purge_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, PurgeCallback) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    if quiet {
        let noop_callback: PurgeCallback =
            Arc::new(move |_event: PurgeEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (download_pb_opt, noop_callback);
    }

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
                        let pb = StyledProgressBar::new(&multi_progress);
                        pb.set_style(get_default_steps_style());
                        pb.set_message("Verifying pads...".to_string());
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
            Ok::<bool, LibError>(true)
        })
    });

    (download_pb_opt, callback)
}
