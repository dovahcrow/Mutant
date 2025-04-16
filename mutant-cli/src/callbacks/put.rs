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
    // Restore res_pb_opt
    res_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    upload_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    confirm_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    multi_progress: MultiProgress,
    total_chunks: Arc<Mutex<usize>>, // Store total chunks for progress updates
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (
    // Restore return tuple for three bars
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    PutCallback,
) {
    // Restore initialization for three bars
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_chunks_arc = Arc::new(Mutex::new(0usize));

    if quiet {
        let noop_callback: PutCallback =
            Box::new(move |_event: PutEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));
        // Restore return for three bars
        return (res_pb_opt, upload_pb_opt, confirm_pb_opt, noop_callback);
    }

    let context = PutCallbackContext {
        res_pb_opt: res_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        confirm_pb_opt: confirm_pb_opt.clone(),
        multi_progress: multi_progress.clone(),
        total_chunks: total_chunks_arc.clone(),
    };

    let ctx_clone = context.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        Box::pin(async move {
            match event {
                PutEvent::Starting { total_chunks } => {
                    debug!("Put Callback: Starting - Total chunks: {}", total_chunks);
                    *ctx.total_chunks.lock().await = total_chunks;
                    let total_u64 = total_chunks as u64;

                    // Initialize Acquisition Bar
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    let res_pb = res_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Acquiring pads...".to_string());
                        pb
                    });
                    res_pb.set_length(total_u64);
                    res_pb.set_position(0); // Start at 0
                    drop(res_pb_guard);

                    // Initialize Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Writing chunks...".to_string());
                        pb
                    });
                    upload_pb.set_length(total_u64);
                    upload_pb.set_position(0);
                    drop(upload_pb_guard);

                    // Initialize Confirmation Bar
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Confirming pads...".to_string()); // Keep name
                        pb
                    });
                    confirm_pb.set_length(total_u64);
                    confirm_pb.set_position(0);
                    drop(confirm_pb_guard);

                    Ok::<bool, LibError>(true)
                }
                PutEvent::PadReserved { count } => {
                    // Restore increment for Acquisition Bar
                    debug!(
                        "Put Callback: Received PadReserved, incrementing acquisition bar by {}",
                        count
                    );
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(count as u64);
                            // Check if the bar is now full and update message
                            if pb.position() >= pb.length().unwrap_or(0) {
                                pb.set_message("Pads acquired.".to_string());
                            }
                        }
                    } else {
                        warn!("Put Callback: PadReserved event but acquisition bar doesn't exist.");
                    }
                    drop(res_pb_guard);
                    Ok::<bool, LibError>(true)
                }
                PutEvent::ChunkWritten { chunk_index: _ } => {
                    // Increment Upload Bar
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
                    // Clear all three bars
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

    // Return all three Arcs
    (res_pb_opt, upload_pb_opt, confirm_pb_opt, callback)
}
