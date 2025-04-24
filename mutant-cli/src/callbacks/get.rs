use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, trace};
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{GetCallback, GetEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

// Get the specific styles needed
use super::progress::{get_default_spinner_style, get_default_steps_style};

pub fn create_get_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, GetCallback) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    if quiet {
        let noop_callback: GetCallback =
            Arc::new(move |_event: GetEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (download_pb_opt, noop_callback);
    }

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: GetCallback = Arc::new(move |event: GetEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        Box::pin(async move {
            match event {
                GetEvent::Starting { total_chunks } => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        // Switch to determinate style now that we have the total
                        pb.set_style(get_default_steps_style());
                        pb.set_length(total_chunks as u64);
                        pb.set_position(0);
                        pb.set_message("Fetching chunks...".to_string());
                        trace!(
                            "Get Callback: Starting - Switched to determinate, length {}, position 0",
                            total_chunks
                        );
                    } else {
                        // Should ideally not happen if IndexLookup was called first, but handle defensively
                        error!(
                            "Get Callback: Starting event received but progress bar does not exist."
                        );
                        // Attempt to create it now (though it missed the IndexLookup state)
                        let _ = pb_guard.get_or_insert_with(|| {
                            let pb = StyledProgressBar::new(&multi_progress);
                            pb.set_style(get_default_steps_style());
                            pb.set_message("Fetching chunks...".to_string());
                            pb.set_length(total_chunks as u64);
                            pb.set_position(0);
                            pb
                        });
                        trace!(
                            "Get Callback: Starting - Created progress bar directly, length {}, position 0",
                            total_chunks
                        );
                    }
                    drop(pb_guard);
                }
                GetEvent::ChunkFetched => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Get Callback: ChunkFetched event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
                GetEvent::Complete => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.take() {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                            trace!("Get Callback: Complete - Progress bar finished and cleared.");
                        }
                    } else {
                        trace!(
                            "Get Callback: Complete event received but progress bar was already finished or never existed."
                        );
                    }
                    drop(pb_guard);
                }
            }
            Ok::<bool, LibError>(true)
        })
    });

    (download_pb_opt, callback)
}
