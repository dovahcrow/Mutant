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
    commit_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    total_bytes_for_upload: Arc<Mutex<u64>>,
    commit_counter_arc: Arc<Mutex<u64>>,
    multi_progress: MultiProgress,
    cyan: Style,
    blue: Style,
    green: Style,
    red: Style,
    yellow: Style,
    magenta: Style,
    separator_style: Style,
}

pub fn create_put_callback(
    multi_progress: &MultiProgress,
) -> (
    // Reservation progress bar (used before confirmation)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Upload progress bar (Tracks UploadProgress)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Commit progress bar (Tracks ScratchpadCommitComplete)
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Shared commit counter (created by caller, passed into callback)
    Arc<Mutex<u64>>,
    // The actual callback closure
    PutCallback,
) {
    let reservation_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let commit_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let total_bytes_for_upload = Arc::new(Mutex::new(0u64));
    let commit_counter_arc = Arc::new(Mutex::new(0u64));

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: reservation_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        commit_pb_opt: commit_pb_opt.clone(),
        total_bytes_for_upload: total_bytes_for_upload.clone(),
        commit_counter_arc: commit_counter_arc.clone(),
        multi_progress: multi_progress.clone(),
        cyan: Style::new().fg(Color::Cyan),
        blue: Style::new().fg(Color::Blue),
        green: Style::new().fg(Color::Green),
        red: Style::new().fg(Color::Red),
        yellow: Style::new().fg(Color::Yellow),
        magenta: Style::new().fg(Color::Magenta),
        separator_style: Style::new().fg(Color::DarkGray),
    };

    // Clone the context for the callback
    let ctx_clone = context.clone();

    let callback: PutCallback = Box::new(move |event: PutEvent| {
        let ctx = ctx_clone.clone();

        async move {
            match event {
                PutEvent::ReservingScratchpads { needed: _ } => Ok(true),
                PutEvent::ReservingPads { count } => {
                    debug!("Received ReservingPads: count={}", count);
                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().await;
                    let pb = res_pb_opt_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Reserving scratchpad(s)...".to_string());
                        pb
                    });
                    pb.set_length(count);
                    pb.set_position(0);
                    Ok(true)
                },
                PutEvent::PadReserved { index: _, total: _, result: _ } => {
                    debug!("Received PadReserved (ignored by progress bar)");
                    Ok(true)
                },
                PutEvent::ConfirmReservation {..} => {
                    warn!("Received ConfirmReservation event, but it is no longer handled by the CLI callback.");
                    Ok(true)
                },
                PutEvent::ReservationProgress { current, total } => {
                    debug!("Received ReservationProgress: current={}, total={}", current, total);

                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().await;
                    if let Some(res_pb) = res_pb_opt_guard.as_mut() {
                        if res_pb.length() != Some(total) {
                            warn!("ReservationProgress: Bar length ({:?}) differs from event total ({}). Resetting length.", res_pb.length(), total);
                            res_pb.set_length(total);
                        }

                        if total > 0 {
                            debug!("Setting reservation bar position to {}", current);
                            res_pb.set_position(current);
                            if current >= total && !res_pb.is_finished() {
                                debug!("Reservation progress complete ({} >= {}), setting final message.", current, total);
                                res_pb.set_message("Reservation complete.".to_string());
                                // Don't finish or clear yet
                            }
                        } else if !res_pb.is_finished() {
                            // If total is 0, set final message immediately
                            debug!("ReservationProgress: Total is 0, setting final message.");
                            res_pb.set_message("Reservation complete (0 needed).".to_string());
                            // Don't finish or clear yet
                        }
                    } else {
                         warn!("ReservationProgress event received but reservation progress bar does not exist.");
                    }
                    Ok(true)
                },
                PutEvent::StartingUpload { total_bytes } => {
                    *ctx.total_bytes_for_upload.lock().await = total_bytes;

                    // Initialize Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().await;
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        StyledProgressBar::new(&ctx.multi_progress)
                    });
                    upload_pb.set_style(super::progress::get_default_bytes_style());
                    upload_pb.set_length(total_bytes);
                    upload_pb.set_position(0);
                    upload_pb.set_message("Uploading...".to_string());

                    Ok(true)
                },
                PutEvent::UploadProgress { bytes_written, total_bytes: _ } => {
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().await.as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_position(bytes_written);
                        }
                    } else {
                        warn!("UploadProgress event received but upload progress bar does not exist.");
                    }
                    Ok(true)
                },
                PutEvent::UploadFinished => {
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().await.as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_message("Upload complete. Committing...".to_string());
                            if let Some(len) = upload_pb.length() {
                                upload_pb.set_position(len);
                            }
                            debug!("UploadFinished event: Marked upload bar as finished, but kept visible.");
                        } else {
                            debug!("UploadFinished event: Upload progress bar was already finished.");
                        }
                    } else {
                        warn!("UploadFinished event received but upload progress bar was already removed/gone.");
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
                    if let Some(commit_pb) = ctx.commit_pb_opt.lock().await.take() {
                        if !commit_pb.is_finished() {
                            commit_pb.finish_and_clear();
                            debug!("StoreComplete event: Finished and cleared commit progress bar.");
                        } else {
                            commit_pb.finish_and_clear();
                            debug!("StoreComplete event: Cleared finished commit progress bar.");
                        }
                    }
                    Ok(true)
                },
                PutEvent::ScratchpadUploadComplete { index: _, total: _ } => {
                    debug!("Received ScratchpadUploadComplete (ignored)");
                    Ok(true)
                },
                PutEvent::ScratchpadCommitComplete { index: _, total } => {
                    // Use the shared commit counter from context
                    let current_committed = *ctx.commit_counter_arc.lock().await;
                    debug!(
                        "Callback: Received ScratchpadCommitComplete - Current: {}, Total: {}",
                        current_committed,
                        total
                    );
                    let mut commit_pb_guard = ctx.commit_pb_opt.lock().await;
                    let commit_pb = commit_pb_guard.get_or_insert_with(|| {
                        debug!("Callback: Initializing commit pads progress bar ({} total)", total);
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        if total > 0 { pb.set_length(total); }
                        pb.set_message("Confirming pads...".to_string());
                        pb.reset();
                        pb
                    });

                    if !commit_pb.is_finished() {
                        debug!(
                            "Callback: Setting commit pads progress bar position to {}",
                            current_committed // Use the counter
                        );
                        if commit_pb.length().is_none() || current_committed <= commit_pb.length().unwrap_or(0) {
                             commit_pb.set_position(current_committed); // Use the counter
                        } else {
                            warn!("Callback: Tried to set commit bar position {} beyond length {:?}", current_committed, commit_pb.length());
                        }
                        if total > 0 && current_committed >= total {
                            debug!("Commit progress complete ({} >= {}), setting final message.", current_committed, total);
                            commit_pb.set_message("Confirmation complete.".to_string());
                            // Don't finish or clear yet
                        }
                    } else {
                        debug!("Callback: Commit pads progress bar is already finished.");
                    }
                    Ok(true)
                }
            }
        }
        .boxed()
    });

    (
        reservation_pb_opt,
        upload_pb_opt,
        commit_pb_opt,
        commit_counter_arc,
        callback,
    )
}
