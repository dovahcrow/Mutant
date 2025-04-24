use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, trace};
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{SyncCallback, SyncEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

// Get the specific styles needed
use super::progress::get_default_steps_style;

pub fn create_sync_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, SyncCallback) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    if quiet {
        let noop_callback: SyncCallback =
            Arc::new(move |_event: SyncEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (download_pb_opt, noop_callback);
    }

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: SyncCallback = Arc::new(move |event: SyncEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        Box::pin(async move {
            match event {
                SyncEvent::FetchingRemoteIndex => {
                    let mut pb_guard = pb_arc.lock().await;
                    let _ = pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new(&multi_progress);
                        pb.set_style(get_default_steps_style());
                        pb.set_message("Fetching remote index...".to_string());
                        pb.set_length(3);
                        pb.set_position(0);
                        pb
                    });

                    drop(pb_guard);
                }
                SyncEvent::Merging => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Sync Callback: Merging event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
                SyncEvent::PushingRemoteIndex => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Sync Callback: PushingRemoteIndex event received but progress bar does not exist."
                        );
                    }
                }
                SyncEvent::Complete => {
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
