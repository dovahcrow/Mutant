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
    multi_progress: MultiProgress,
    total_chunks: Arc<Mutex<usize>>, // Store total chunks for progress updates
    quiet: bool,
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (
    // Return Arcs for potential external use (though maybe not needed now)
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    // The actual callback closure
    PutCallback,
) {
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_chunks_arc = Arc::new(Mutex::new(0usize));

    // Return early with no-op callback and dummy Arcs if quiet
    if quiet {
        // Use Box::pin for Send + Sync
        // The callback needs to be a Boxed Fn returning a Pinned Future
        let noop_callback: PutCallback =
            Box::new(move |_event: PutEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (res_pb_opt, upload_pb_opt, confirm_pb_opt, noop_callback);
    }

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: res_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        confirm_pb_opt: confirm_pb_opt.clone(),
        multi_progress: multi_progress.clone(),
        total_chunks: total_chunks_arc.clone(),
        quiet: quiet,
    };

    // Clone the context for the callback
    let ctx_clone = context.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        // Return a Pinned, Boxed Future that is Send + Sync
        Box::pin(async move {
            // Lock mutexes, perform operations, then drop guards before awaits or returns
            match event {
                PutEvent::Starting { total_chunks } => {
                    debug!("Put Callback: Starting - Total chunks: {}", total_chunks);
                    *ctx.total_chunks.lock().await = total_chunks; // Store total chunks
                    let total_u64 = total_chunks as u64;

                    // Initialize and immediately finish Acquisition Bar
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    let res_pb = res_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Acquiring pads...".to_string());
                        pb
                    });
                    res_pb.set_length(total_u64);
                    // Set position to length to mark as complete immediately
                    // as acquisition (reuse/generate) happens before first write event.
                    res_pb.set_position(total_u64);
                    drop(res_pb_guard);

                    // Initialize Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        // Correct message for upload bar
                        pb.set_message("Writing chunks...".to_string());
                        pb
                    });
                    upload_pb.set_length(total_u64);
                    upload_pb.set_position(0); // Start at 0
                    drop(upload_pb_guard);

                    // Initialize Confirmation Bar
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Confirming writes...".to_string());
                        pb
                    });
                    confirm_pb.set_length(total_u64);
                    confirm_pb.set_position(0); // Start at 0
                    drop(confirm_pb_guard);

                    Ok::<bool, LibError>(true)
                }
                PutEvent::PadReserved { count: _ } => {
                    // Do nothing here for progress bars.
                    debug!("Put Callback: PadReserved event received (ignored for progress bar).");
                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkWritten { chunk_index: _ } => {
                    // Only increment Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(pb) = upload_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        warn!("Put Callback: ChunkWritten event but upload bar doesn't exist.");
                    }
                    drop(upload_pb_guard);
                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkConfirmed { chunk_index: _ } => {
                    // Only increment Confirmation Bar
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        warn!(
                            "Put Callback: ChunkConfirmed event but confirmation bar doesn't exist."
                        );
                    }
                    drop(confirm_pb_guard);
                    Ok::<bool, LibError>(true)
                }
                PutEvent::SavingIndex => {
                    // This event is likely deprecated/unused now, log but do nothing to bars.
                    warn!("Put Callback: Received unexpected SavingIndex event.");
                    Ok::<bool, LibError>(true)
                }
                PutEvent::Complete => {
                    debug!("Put Callback: Complete event received, clearing bars");
                    // Finish and clear all bars
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(res_pb_guard);

                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(pb) = upload_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(upload_pb_guard);
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(confirm_pb_guard);
                    Ok::<bool, LibError>(true)
                }
            }
        })
    });

    // Return the Arcs needed by handle_put
    (res_pb_opt, upload_pb_opt, confirm_pb_opt, callback)
}
