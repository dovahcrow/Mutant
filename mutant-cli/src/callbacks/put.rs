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
    create_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    confirm_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    multi_progress: MultiProgress,
    total_chunks: Arc<Mutex<usize>>, // Store total chunks for progress updates
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    PutCallback,
) {
    let create_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_chunks_arc = Arc::new(Mutex::new(0usize));

    // Return early with no-op callback and dummy Arcs if quiet
    if quiet {
        let noop_callback: PutCallback =
            Box::new(move |_event: PutEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        return (create_pb_opt, confirm_pb_opt, noop_callback);
    }

    // Create the context instance
    let context = PutCallbackContext {
        create_pb_opt: create_pb_opt.clone(),
        confirm_pb_opt: confirm_pb_opt.clone(),
        multi_progress: multi_progress.clone(),
        total_chunks: total_chunks_arc.clone(),
    };

    // Clone the context for the callback
    let ctx_clone = context.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        // Return a Pinned, Boxed Future that is Send + Sync
        Box::pin(async move {
            match event {
                PutEvent::Starting { total_chunks } => {
                    debug!("Put Callback: Starting - Total chunks: {}", total_chunks);
                    *ctx.total_chunks.lock().await = total_chunks;
                    let total_u64 = total_chunks as u64;

                    // Initialize Create Bar (replaces upload bar)
                    let mut create_pb_guard = ctx.create_pb_opt.lock().await;
                    let create_pb = create_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Creating pads...".to_string());
                        pb
                    });
                    create_pb.set_length(total_u64);
                    create_pb.set_position(0);
                    drop(create_pb_guard);

                    // Initialize Confirmation Bar
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Confirming pads...".to_string());
                        pb
                    });
                    confirm_pb.set_length(total_u64);
                    confirm_pb.set_position(0);
                    drop(confirm_pb_guard);

                    Ok::<bool, LibError>(true)
                }
                PutEvent::PadReserved { count: _ } => {
                    // Do nothing here for progress bars.
                    debug!("Put Callback: PadReserved event received (ignored for progress bar).");
                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkWritten { chunk_index: _ } => {
                    // Increment Create Bar
                    let mut create_pb_guard = ctx.create_pb_opt.lock().await;
                    if let Some(pb) = create_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(1);
                        }
                    } else {
                        warn!("Put Callback: ChunkWritten event but create bar doesn't exist.");
                    }
                    drop(create_pb_guard);
                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkConfirmed { chunk_index: _ } => {
                    // Increment Confirmation Bar
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
                    warn!("Put Callback: Received unexpected SavingIndex event.");
                    Ok::<bool, LibError>(true)
                }
                PutEvent::Complete => {
                    debug!("Put Callback: Complete event received, clearing bars");
                    // Finish and clear both bars
                    let mut create_pb_guard = ctx.create_pb_opt.lock().await;
                    if let Some(pb) = create_pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    drop(create_pb_guard);

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

    // Return the two Arcs needed by handle_put
    (create_pb_opt, confirm_pb_opt, callback)
}
