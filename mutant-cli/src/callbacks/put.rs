use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{PutCallback, PutEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
struct PutCallbackContext {
    res_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    upload_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    confirm_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    multi_progress: MultiProgress,
    total_chunks: Arc<Mutex<usize>>,
}

#[allow(clippy::type_complexity)]
pub fn create_put_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    Arc<Mutex<Option<StyledProgressBar>>>,
    PutCallback,
) {
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_chunks_arc = Arc::new(Mutex::new(0usize));

    if quiet {
        let noop_callback: PutCallback =
            Box::new(move |_event: PutEvent| Box::pin(async move { Ok::<bool, LibError>(true) }));

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
                PutEvent::Starting {
                    total_chunks,
                    initial_written_count,
                    initial_confirmed_count,
                } => {
                    debug!(
                        "Put Callback: Starting - Total chunks: {}, Initial Written: {}, Initial Confirmed: {}",
                        total_chunks, initial_written_count, initial_confirmed_count
                    );
                    *ctx.total_chunks.lock().await = total_chunks;
                    let total_u64 = total_chunks as u64;

                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    let res_pb = res_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Acquiring pads...".to_string());
                        pb
                    });
                    res_pb.set_length(total_u64);
                    res_pb.set_position(initial_written_count as u64);
                    drop(res_pb_guard);

                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Writing chunks...".to_string());
                        pb
                    });
                    upload_pb.set_length(total_u64);
                    upload_pb.set_position(initial_written_count as u64);
                    drop(upload_pb_guard);

                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Confirming pads...".to_string());
                        pb
                    });
                    confirm_pb.set_length(total_u64);
                    confirm_pb.set_position(initial_confirmed_count as u64);
                    drop(confirm_pb_guard);

                    Ok::<bool, LibError>(true)
                }
                PutEvent::PadReserved { count } => {
                    debug!(
                        "Put Callback: Received PadReserved, incrementing acquisition bar by {}",
                        count
                    );
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.inc(count as u64);

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

    (res_pb_opt, upload_pb_opt, confirm_pb_opt, callback)
}
