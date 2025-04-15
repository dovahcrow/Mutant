use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{debug, warn};
// Use the new top-level re-exports
use mutant_lib::{PutCallback, PutEvent, error::Error as LibError};
use std::sync::Arc;
use tokio::sync::Mutex;

// Define the context struct to hold shared state and styles
#[derive(Clone)]
struct PutCallbackContext {
    res_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    upload_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    confirm_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    total_bytes_for_upload: Arc<Mutex<u64>>,
    multi_progress: MultiProgress,
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (
    // Reservation progress bar (used before confirmation)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Upload progress bar (Tracks UploadProgress)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Confirm progress bar (Tracks ScratchpadCommitComplete)
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<u64>>,
    // The actual callback closure
    PutCallback,
) {
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_bytes_for_upload = Arc::new(Mutex::new(0u64));
    let confirm_counter_arc = Arc::new(Mutex::new(0u64));

    // Return early with no-op callback and dummy Arcs if quiet
    if quiet {
        // Use Box::pin for Send + Sync
        // The callback needs to be a Boxed Fn returning a Pinned Future
        let noop_callback: PutCallback =
            Box::new(move |_event: PutEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (
            res_pb_opt,
            upload_pb_opt,
            confirm_pb_opt,
            confirm_counter_arc,
            noop_callback,
        );
    }

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: res_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        confirm_pb_opt: confirm_pb_opt.clone(),
        total_bytes_for_upload: total_bytes_for_upload.clone(),
        multi_progress: multi_progress.clone(),
    };

    // Clone the context for the callback
    let ctx_clone = context.clone();
    let _confirm_counter_clone = confirm_counter_arc.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();
        // Note: confirm_counter_arc/clone are no longer used here with the new events

        // Return a Pinned, Boxed Future that is Send + Sync
        Box::pin(async move {
            // Lock mutexes, perform operations, then drop guards before awaits or returns
            match event {
                PutEvent::Starting { total_chunks } => {
                    debug!("Put Callback: Starting - Total chunks: {}", total_chunks);

                    // Initialize Upload Bar (represents chunk writes)
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Writing chunks...".to_string());
                        pb
                    });
                    upload_pb.set_length(total_chunks as u64);
                    upload_pb.set_position(0);
                    drop(upload_pb_guard); // Drop guard

                    // Clear other potential bars (no longer used)
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(res_pb_guard);
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(confirm_pb_guard);

                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkWritten { chunk_index } => {
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(upload_pb) = upload_pb_guard.as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_position((chunk_index + 1) as u64);
                        }
                    } else {
                        warn!("Put Callback: ChunkWritten event but upload bar doesn't exist.");
                    }
                    drop(upload_pb_guard); // Drop guard
                    Ok::<bool, LibError>(true)
                }
                PutEvent::SavingIndex => {
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(upload_pb) = upload_pb_guard.as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_message("Saving index...".to_string());
                        }
                    } else {
                        warn!("Put Callback: SavingIndex event but upload bar doesn't exist.");
                    }
                    drop(upload_pb_guard); // Drop guard
                    Ok::<bool, LibError>(true)
                }
                PutEvent::Complete => {
                    debug!("Put Callback: Complete event received, clearing bars");
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(pb) = upload_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(upload_pb_guard);
                    // Clear others just in case
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(res_pb_guard);
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(confirm_pb_guard);
                    Ok::<bool, LibError>(true)
                } // REMOVED old variants: ReservingPads, StartingUpload, UploadProgress, StoreComplete, PadConfirmed, PadCreateSuccess, Finished, ErrorOccurred
            }
        })
    });

    // Return the Arcs needed by handle_put (res and confirm are dummies now)
    (
        res_pb_opt,
        upload_pb_opt,
        confirm_pb_opt,
        confirm_counter_arc,
        callback,
    )
}
