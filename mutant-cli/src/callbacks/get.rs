use super::progress::StyledProgressBar;

use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::Error as LibError;
use mutant_lib::{GetCallback, GetEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn create_get_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, GetCallback) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    if quiet {
        let noop_callback: GetCallback =
            Box::new(move |_event: GetEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (download_pb_opt, noop_callback);
    }

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: GetCallback = Box::new(move |event: GetEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        Box::pin(async move {
            match event {
                GetEvent::Starting { total_chunks } => {
                    let mut pb_guard = pb_arc.lock().await;
                    let pb = pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new(&multi_progress);
                        pb.set_style(super::progress::get_default_steps_style());
                        pb.set_message("Fetching chunks...".to_string());
                        pb
                    });
                    pb.set_length(total_chunks as u64);
                    pb.set_position(0);
                    debug!(
                        "Get Callback: Starting - Set length to {} chunks",
                        total_chunks
                    );
                    drop(pb_guard);
                }
                GetEvent::ChunkFetched { chunk_index } => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.set_position((chunk_index + 1) as u64);
                        }
                    } else {
                        warn!(
                            "Get Callback: ChunkFetched event received but progress bar does not exist."
                        );
                    }
                    drop(pb_guard);
                }
                GetEvent::Reassembling => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.set_message("Reassembling data...".to_string());
                        }
                    }
                    drop(pb_guard);
                }
                GetEvent::Complete => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.take() {
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                            debug!("Get Callback: Complete - Progress bar finished and cleared.");
                        }
                    } else {
                        debug!(
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
