use super::progress::StyledProgressBar;
use futures::future::FutureExt;
use indicatif::MultiProgress;
use log::{debug, warn};
use mutant_lib::events::{PutCallback, PutEvent};
use nu_ansi_term::{Color, Style};
use std::sync::Arc;
use tokio::sync::Mutex;

// Define the context struct to hold shared state and styles
#[derive(Clone)]
struct PutCallbackContext {
    res_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    upload_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    confirm_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    total_bytes_for_upload: Arc<Mutex<u64>>,
    uploaded_bytes_counter: Arc<Mutex<u64>>,
    confirm_counter_arc: Arc<Mutex<u64>>,
    total_pads: Arc<Mutex<u64>>,
    multi_progress: MultiProgress,
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
) -> (
    // Reservation progress bar (used before confirmation)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Upload progress bar (Tracks UploadProgress)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Confirm progress bar (Tracks ScratchpadCommitComplete)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Shared confirm counter (created by caller, passed into callback)
    Arc<Mutex<u64>>,
    // The actual callback closure
    PutCallback,
) {
    let res_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirm_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_bytes_for_upload = Arc::new(Mutex::new(0u64));
    let uploaded_bytes_counter = Arc::new(Mutex::new(0u64));
    let confirm_counter_arc = Arc::new(Mutex::new(0u64));
    let total_pads = Arc::new(Mutex::new(0u64));

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: res_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        confirm_pb_opt: confirm_pb_opt.clone(),
        total_bytes_for_upload: total_bytes_for_upload.clone(),
        uploaded_bytes_counter: uploaded_bytes_counter.clone(),
        confirm_counter_arc: confirm_counter_arc.clone(),
        total_pads: total_pads.clone(),
        multi_progress: multi_progress.clone(),
    };

    // Clone the context for the callback
    let ctx_clone = context.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        async move {
            match event {
                PutEvent::ReservingPads { count } => {
                    debug!("Received ReservingPads: count={}", count);
                    *ctx.total_pads.lock().await = count;
                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().await;
                    let pb = res_pb_opt_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Initializing pads...".to_string());
                        pb
                    });
                    pb.set_length(count);
                    pb.set_position(0);
                    Ok(true)
                },
                PutEvent::StartingUpload { total_bytes } => {
                    *ctx.total_bytes_for_upload.lock().await = total_bytes;
                    *ctx.uploaded_bytes_counter.lock().await = 0;

                    // Initialize Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        StyledProgressBar::new(&ctx.multi_progress)
                    });
                    upload_pb.set_style(super::progress::get_default_bytes_style());
                    upload_pb.set_length(total_bytes);
                    upload_pb.set_position(0);
                    upload_pb.set_message("Uploading...".to_string());

                    // Initialize Confirmation Bar using stored total_pads
                    let total_pads_count = *ctx.total_pads.lock().await;
                    *ctx.confirm_counter_arc.lock().await = 0;
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    let confirm_pb = confirm_pb_guard.get_or_insert_with(|| {
                        StyledProgressBar::new_for_steps(&ctx.multi_progress)
                    });
                    confirm_pb.set_message("Confirming...".to_string());
                    confirm_pb.set_length(total_pads_count);
                    confirm_pb.set_position(0);

                    Ok(true)
                },
                PutEvent::UploadProgress { bytes_written, total_bytes: _ } => {
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().await.as_mut() {
                        if !upload_pb.is_finished() {
                            let current_pos = upload_pb.position();
                            if bytes_written > current_pos {
                                upload_pb.set_position(bytes_written);
                            }
                        }
                    } else {
                        warn!("UploadProgress event received but upload progress bar does not exist.");
                    }
                    Ok(true)
                },
                PutEvent::StoreComplete => {
                    if let Some(res_pb) = ctx.res_pb_opt.lock().await.take() {
                        if !res_pb.is_finished() {
                            res_pb.finish_and_clear();
                            debug!("StoreComplete event: Finished and cleared reservation bar.");
                        } else {
                             res_pb.finish_and_clear();
                            debug!("StoreComplete event: Cleared finished reservation bar.");
                        }
                    }
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().await.take() {
                        if !upload_pb.is_finished() {
                            upload_pb.finish_and_clear();
                            debug!("StoreComplete event: Force-finished and cleared upload bar.");
                        } else {
                            upload_pb.finish_and_clear();
                            debug!("StoreComplete event: Cleared finished upload bar.");
                        }
                    }
                    if let Some(confirm_pb) = ctx.confirm_pb_opt.lock().await.take() {
                        if !confirm_pb.is_finished() {
                            confirm_pb.finish_and_clear();
                            debug!("StoreComplete event: Finished and cleared confirmation bar.");
                        } else {
                            confirm_pb.finish_and_clear();
                            debug!("StoreComplete event: Cleared finished confirmation bar.");
                        }
                    }
                    Ok(true)
                },
                PutEvent::PadConfirmed { current, total } => {
                    debug!(
                        "Callback: Received PadConfirmed - Current: {}, Total: {}",
                        current, total
                    );

                    // Update Confirmation Bar
                    let mut confirm_pb_guard = ctx.confirm_pb_opt.lock().await;
                    // Bar should be initialized by StartingUpload
                    if let Some(confirm_pb) = confirm_pb_guard.as_mut() {
                        if !confirm_pb.is_finished() {
                            // Check and set length if it's currently 0 (e.g., no reservations)
                            let mut bar_total = confirm_pb.length().unwrap_or(0);
                            if bar_total == 0 && total > 0 {
                                debug!("PadConfirmed: Confirm bar length is 0, setting from event total ({}).", total);
                                confirm_pb.set_length(total);
                                bar_total = total; // Update local variable
                            }

                            // Use the 'current' value directly from the event
                            let new_pos = current; // Use event value

                            // Set position based on event's 'current', capped at bar_total
                            if bar_total > 0 {
                                let display_pos = std::cmp::min(new_pos, bar_total);
                                confirm_pb.set_position(display_pos);

                                if display_pos >= bar_total {
                                    debug!("Confirmation progress complete ({} >= {}), setting final message.", display_pos, bar_total);
                                    confirm_pb.set_message("Confirmation complete.".to_string());
                                    // Don't finish/clear yet
                                }
                            } else {
                                // If bar_total is still 0, keep position at 0
                                confirm_pb.set_position(0);
                                if new_pos > 0 {
                                    // Use current (from event) in warning message
                                    warn!("PadConfirmed received but confirmation bar total is 0. Event Current: {}", current);
                                }
                            }
                            debug!(
                                "Callback: PadConfirmed updated - Event Current: {}, Bar Total: {}, New Position: {}",
                                current, // Log event's current value
                                bar_total,
                                confirm_pb.position()
                            );
                        } else {
                            debug!("Callback: Confirmation bar is already finished.");
                        }
                    } else {
                        warn!("PadConfirmed event received but confirmation progress bar does not exist.");
                    }
                    Ok(true)
                },
                PutEvent::PadCreateSuccess { current, total } => {
                    debug!(
                        "Callback: Received PadCreateSuccess - Current: {}, Total: {}",
                        current, total
                    );
                    if let Some(res_pb) = ctx.res_pb_opt.lock().await.as_mut() {
                        if !res_pb.is_finished() {
                            if res_pb.length() != Some(total) {
                                warn!(
                                    "PadCreateSuccess: Res bar length ({:?}) differs from event total ({}). Resetting length.",
                                    res_pb.length(), total
                                );
                                res_pb.set_length(total);
                            }
                            if current <= res_pb.length().unwrap_or(0) {
                                res_pb.set_position(current);
                            } else {
                                warn!("PadCreateSuccess: Tried to set res bar position {} beyond length {:?}", current, res_pb.length());
                            }
                            if total > 0 && current >= total {
                                res_pb.set_message("Reservation complete.".to_string());
                            }
                        }
                    } else {
                        warn!("PadCreateSuccess event received but reservation progress bar does not exist.");
                    }
                    Ok(true)
                },
            }
        }
        .boxed()
    });

    (
        res_pb_opt,
        upload_pb_opt,
        confirm_pb_opt,
        confirm_counter_arc,
        callback,
    )
}
