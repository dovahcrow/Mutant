use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{error, trace, warn};
use mutant_client::ProgressReceiver;
use mutant_protocol::{SyncCallback, SyncEvent, TaskProgress};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn create_sync_progress(mut progress_rx: ProgressReceiver, multi_progress: &MultiProgress) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

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
                        let pb = StyledProgressBar::new_for_steps(&multi_progress);
                        pb.set_message("Fetching remote index...".to_string());
                        pb.set_length(4);
                        pb.set_position(0);
                        pb
                    });

                    drop(pb_guard);
                }
                SyncEvent::Merging => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.set_message("Merging remote index...".to_string());
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
                            pb.set_message("Pushing remote index...".to_string());
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Sync Callback: PushingRemoteIndex event received but progress bar does not exist."
                        );
                    }
                }
                SyncEvent::VerifyingRemoteIndex => {
                    let mut pb_guard = pb_arc.lock().await;
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.set_message("Verifying remote index...".to_string());
                            pb.inc(1);
                        }
                    } else {
                        error!(
                            "Sync Callback: VerifyingRemoteIndex event received but progress bar does not exist."
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
            Ok(true)
        })
    });

    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            match progress {
                Ok(TaskProgress::Sync(event)) => {
                    callback(event.clone()).await.unwrap();
                }
                Ok(_) => warn!("Unexpected progress type"),
                Err(e) => error!("Progress error: {:?}", e),
            }
        }
    });
}
