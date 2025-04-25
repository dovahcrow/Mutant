use super::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{debug, info, warn};
use mutant_protocol::{PutCallback, PutEvent};
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
    info!("Creating put callback with quiet={}", quiet);
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_chunks_arc = Arc::new(Mutex::new(0usize));

    if quiet {
        info!("Quiet mode enabled, using noop callback");
        let noop_callback: PutCallback = Arc::new(move |_event: PutEvent| {
            Box::pin(async move { Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true) })
        });

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

    let callback: PutCallback = Arc::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        Box::pin(async move {
            match event {
                PutEvent::Starting {
                    total_chunks,
                    initial_written_count,
                    initial_confirmed_count,
                    chunks_to_reserve,
                } => {
                    info!(
                        "Starting put operation - Total: {}, Written: {}, Confirmed: {}, To Reserve: {}",
                        total_chunks, initial_written_count, initial_confirmed_count, chunks_to_reserve
                    );
                    *ctx.total_chunks.lock().await = total_chunks;
                    let total_u64 = total_chunks as u64;

                    if chunks_to_reserve > 0 {
                        let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                        let res_pb = res_pb_guard.get_or_insert_with(|| {
                            info!("Creating reservation progress bar");
                            let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                            pb.set_message("Acquiring pads...".to_string());
                            pb
                        });
                        info!("Setting reservation bar length to {}", chunks_to_reserve);
                        res_pb.set_length(chunks_to_reserve as u64);
                        res_pb.set_position(0);
                        drop(res_pb_guard);
                    }

                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        info!("Creating upload progress bar");
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Writing pads...".to_string());
                        pb
                    });
                    info!("Setting upload bar length to {}", total_u64);
                    upload_pb.set_length(total_u64);
                    upload_pb.set_position(initial_written_count as u64);
                    drop(upload_pb_guard);

                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        info!("Creating confirmation progress bar");
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Confirming pads...".to_string());
                        pb
                    });
                    info!("Setting confirmation bar length to {}", total_u64);
                    confirm_pb.set_length(total_u64);
                    confirm_pb.set_position(initial_confirmed_count as u64);
                    drop(confirm_pb_guard);

                    Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true)
                }
                PutEvent::PadReserved => {
                    info!("Pad reserved event received");
                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            info!("Incrementing reservation bar");
                            pb.inc(1);

                            if pb.position() >= pb.length().unwrap_or(0) {
                                info!("All pads acquired");
                                pb.set_message("Pads acquired.".to_string());
                            }
                        }
                    } else {
                        warn!("PadReserved event but acquisition bar doesn't exist");
                    }
                    drop(res_pb_guard);
                    Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true)
                }
                PutEvent::PadsWritten => {
                    info!("Pads written event received");
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(pb) = upload_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            info!("Incrementing upload bar");
                            pb.inc(1);
                        }
                    } else {
                        warn!("PadsWritten event but upload bar doesn't exist");
                    }
                    drop(upload_pb_guard);
                    Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true)
                }
                PutEvent::PadsConfirmed => {
                    info!("Pads confirmed event received");
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.as_mut() {
                        if !pb.is_finished() {
                            info!("Incrementing confirmation bar");
                            pb.inc(1);
                        }
                    } else {
                        warn!("PadsConfirmed event but confirmation bar doesn't exist");
                    }
                    drop(confirm_pb_guard);
                    Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true)
                }
                PutEvent::Complete => {
                    info!("Complete event received, clearing progress bars");

                    let mut res_pb_guard = ctx.res_pb_opt.lock().await;
                    if let Some(pb) = res_pb_guard.take() {
                        info!("Clearing reservation bar");
                        pb.finish_and_clear();
                    }
                    drop(res_pb_guard);

                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    if let Some(pb) = upload_pb_guard.take() {
                        info!("Clearing upload bar");
                        pb.finish_and_clear();
                    }
                    drop(upload_pb_guard);

                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    if let Some(pb) = confirm_pb_guard.take() {
                        info!("Clearing confirmation bar");
                        pb.finish_and_clear();
                    }
                    drop(confirm_pb_guard);
                    Ok::<bool, Box<dyn std::error::Error + Send + Sync>>(true)
                }
            }
        })
    });

    info!("Put callback created successfully");
    (res_pb_opt, upload_pb_opt, confirm_pb_opt, callback)
}
