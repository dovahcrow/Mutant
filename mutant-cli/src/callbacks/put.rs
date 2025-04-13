use super::progress::StyledProgressBar;
use mutant_lib::error::Error;
use mutant_lib::events::{PutCallback, PutEvent};
use dialoguer::Confirm;
use futures::future::FutureExt;
use humansize::{BINARY, format_size};
use indicatif::MultiProgress;
use log::{debug, warn};
use nu_ansi_term::{Color, Style};
use std::sync::{Arc, Mutex};

// Define the context struct to hold shared state and styles
#[derive(Clone)]
struct PutCallbackContext {
    res_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    upload_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    commit_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    res_confirmed_and_needed: Arc<Mutex<bool>>,
    total_bytes_for_upload: Arc<Mutex<u64>>,
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
    // Flag indicating if reservation was confirmed and needed
    Arc<Mutex<bool>>,
    // The actual callback closure
    PutCallback,
) {
    let reservation_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let commit_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let reservation_confirmed_and_needed = Arc::new(Mutex::new(false));
    let total_bytes_for_upload = Arc::new(Mutex::new(0u64));

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: reservation_pb_opt.clone(),
        upload_pb_opt: upload_pb_opt.clone(),
        commit_pb_opt: commit_pb_opt.clone(),
        res_confirmed_and_needed: reservation_confirmed_and_needed.clone(),
        total_bytes_for_upload: total_bytes_for_upload.clone(),
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
        // Use the cloned context
        let ctx = ctx_clone.clone(); 

        async move {
            match event {
                PutEvent::ReservingScratchpads { needed: _ } => Ok(true),
                PutEvent::ReservingPads { count } => {
                    debug!("Received ReservingPads: count={}", count);
                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().unwrap();
                    let pb = res_pb_opt_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_message("Reserving scratchpad(s)... (Don't panic, might take a while)".to_string());
                        pb
                    });
                    pb.set_length(count);
                    pb.set_position(0);
                    Ok(true)
                },
                PutEvent::PadReserved { index, total, result } => {
                    debug!(
                        "Received PadReserved: index={}, total={}, result={:?}",
                        index, total, result
                    );
                    if let Err(e) = result {
                        warn!("Reservation failed for pad index {}: {}", index, e);
                    }
                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().unwrap();
                    if let Some(res_pb) = res_pb_opt_guard.as_mut() {
                        let current_position = index + 1;
                        if res_pb.length() == Some(total) {
                            res_pb.inc(1);
                            if current_position >= total && !res_pb.is_finished() {
                                debug!(
                                    "Pad reservation complete ({} >= {}), finishing reservation bar.",
                                    current_position, total
                                );
                                res_pb.finish_and_clear();
                                *res_pb_opt_guard = None;
                            }
                        } else {
                            warn!(
                                "PadReserved: Progress bar length ({:?}) does not match total ({})",
                                res_pb.length(),
                                total
                            );
                            res_pb.set_length(total);
                            res_pb.inc(1);
                        }
                    } else {
                        warn!("PadReserved event received but reservation progress bar does not exist.");
                    }
                    Ok(true)
                },
                PutEvent::ConfirmReservation {
                    needed,
                    data_size,
                    total_space,
                    free_space,
                    current_scratchpads,
                    estimated_cost,
                } => {
                    if needed == 0 {
                        debug!("ConfirmReservation called with needed=0, skipping prompt.");
                        return Ok(true);
                    }

                    let data_size_fmt = ctx.cyan.paint(format_size(data_size, BINARY));
                    let total_space_fmt = ctx.blue.paint(format_size(total_space, BINARY));
                    let free_space_fmt = ctx.green.paint(format_size(free_space, BINARY));
                    let used_space_fmt =
                        ctx.cyan.paint(format_size(total_space.saturating_sub(free_space), BINARY));
                    let missing_space = data_size.saturating_sub(free_space);
                    let missing_space_fmt = if missing_space > 0 {
                        ctx.red.paint(format_size(missing_space, BINARY))
                    } else {
                        ctx.green.paint(format_size(missing_space, BINARY))
                    };

                    let current_scratchpads_fmt = ctx.yellow.paint(current_scratchpads.to_string());
                    let needed_fmt = ctx.green.paint(format!("+ {}", needed));

                    let cost_str = estimated_cost
                        .map(|c| ctx.magenta.paint(format!("Cost: {}", c)).to_string())
                        .unwrap_or_else(|| ctx.magenta.paint("Cost: N/A").to_string());

                    let sep = ctx.separator_style.paint(" | ");
                    let prompt_text = format!(
                        "Upload: {}{}\
                         Storage: {} / {} ({} free){}\
                         Missing: {}{}\
                         Pads: {} ({}){}\
                         {}\
                         {}Proceed?",
                        data_size_fmt,
                        sep,
                        used_space_fmt,
                        total_space_fmt,
                        free_space_fmt,
                        sep,
                        missing_space_fmt,
                        sep,
                        current_scratchpads_fmt,
                        needed_fmt,
                        sep,
                        cost_str,
                        sep
                    );

                    let confirmation = Confirm::new()
                        .with_prompt(prompt_text)
                        .interact()
                        .map_err(|e| Error::InternalError(format!("Dialoguer error: {}", e)))?;

                    if confirmation {
                        let mut confirmed_needed_guard = ctx.res_confirmed_and_needed.lock().unwrap();
                        *confirmed_needed_guard = true;

                        let mut res_pb_opt_guard = ctx.res_pb_opt.lock().unwrap();
                        res_pb_opt_guard.get_or_insert_with(|| {
                            let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                            pb.set_message("Reserving scratchpad(s)...".to_string());
                            pb.set_length(needed);
                            pb
                        });

                        Ok(true)
                    } else {
                        eprintln!("Reservation cancelled by user.");
                        Ok(false)
                    }
                }
                PutEvent::ReservationProgress { current, total } => {
                    let confirmed_needed_guard = ctx.res_confirmed_and_needed.lock().unwrap();
                    if !*confirmed_needed_guard { return Ok(true); }
                    drop(confirmed_needed_guard);
                    
                    debug!("Received ReservationProgress: current={}, total={}", current, total);

                    let mut res_pb_opt_guard = ctx.res_pb_opt.lock().unwrap();
                    if let Some(res_pb) = res_pb_opt_guard.as_mut() {
                        if res_pb.length().is_some() && res_pb.length() != Some(0) {
                            debug!("Setting reservation bar position to {}", current);
                            res_pb.set_position(current);
                        } else {
                             warn!("ReservationProgress: Cannot set position {} because length is 0 or None", current);
                        }

                        if total > 0 && current >= total && !res_pb.is_finished() {
                             debug!("Reservation progress complete ({} >= {}), finishing reservation bar.", current, total);
                             res_pb.finish_and_clear();
                        }
                    } else {
                         warn!("ReservationProgress event received but reservation progress bar does not exist.");
                    }
                    Ok(true)
                }
                PutEvent::StartingUpload { total_bytes } => {
                    *ctx.total_bytes_for_upload.lock().unwrap() = total_bytes;

                    // Initialize Upload Bar
                    let mut upload_pb_guard = ctx.upload_pb_opt.lock().unwrap();
                    let upload_pb = upload_pb_guard.get_or_insert_with(|| {
                        StyledProgressBar::new(&ctx.multi_progress)
                    });
                    upload_pb.set_style(super::progress::get_default_bytes_style());
                    upload_pb.set_length(total_bytes);
                    upload_pb.set_position(0);
                    upload_pb.set_message("Uploading...".to_string());

                    Ok(true)
                }
                PutEvent::UploadProgress { bytes_written, total_bytes: _ } => {
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().unwrap().as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_position(bytes_written);
                        }
                    } else {
                        warn!("UploadProgress event received but upload progress bar does not exist.");
                    }
                    Ok(true)
                }
                PutEvent::UploadFinished => {
                    // Mark upload bar as complete but keep it visible until StoreComplete.
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().unwrap().as_mut() {
                        if !upload_pb.is_finished() {
                            upload_pb.set_message("Upload complete. Committing...".to_string());
                            // Ensure it shows 100%
                            if let Some(len) = upload_pb.length() {
                                upload_pb.set_position(len);
                            } else {
                                // If length is None, just update the message. StoreComplete will clear it.
                                debug!("UploadFinished: Length is None, only updating message.");
                            }
                            debug!("UploadFinished event: Marked upload bar as finished, but kept visible.");
                        } else {
                             debug!("UploadFinished event: Upload progress bar was already finished.");
                        }
                    } else {
                        warn!("UploadFinished event received but upload progress bar was already removed/gone.");
                    }
                    Ok(true)
                }
                PutEvent::StoreComplete => {
                    // Finish and clear Upload bar (if it exists and isn't finished)
                    if let Some(upload_pb) = ctx.upload_pb_opt.lock().unwrap().take() {
                        if !upload_pb.is_finished() {
                            upload_pb.finish_and_clear();
                            debug!("StoreComplete event: Force-finished and cleared upload bar.");
                        } else {
                            // Already finished by UploadFinished, just clear it
                            upload_pb.finish_and_clear(); // Ensures clearance even if finish() was called without clear
                            debug!("StoreComplete event: Cleared finished upload bar.");
                        }
                    }
                    // Finish and clear Commit bar
                    if let Some(commit_pb) = ctx.commit_pb_opt.lock().unwrap().take() {
                        if !commit_pb.is_finished() {
                            commit_pb.finish_and_clear();
                            debug!("StoreComplete event: Finished and cleared commit progress bar.");
                        } else {
                            // Already finished (e.g. last commit event), just clear
                            commit_pb.finish_and_clear(); 
                            debug!("StoreComplete event: Cleared finished commit progress bar.");
                        }
                    }
                    Ok(true)
                }
                PutEvent::ScratchpadUploadComplete { index: _, total: _ } => {
                    debug!("Received ScratchpadUploadComplete (ignored in CLI)");
                    Ok(true)
                }
                PutEvent::ScratchpadCommitComplete { index, total } => {
                    debug!(
                        "Callback: Received ScratchpadCommitComplete - Index: {}, Total: {}",
                        index,
                        total
                    );
                    let mut commit_pb_guard = ctx.commit_pb_opt.lock().unwrap();
                    let commit_pb = commit_pb_guard.get_or_insert_with(|| {
                        debug!("Callback: Initializing commit pads progress bar ({} total)", total);
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_length(total);
                        pb.set_message("Confirming pads...".to_string());
                        pb.reset(); // Ensure position starts at 0
                        pb
                    });

                    if !commit_pb.is_finished() {
                        // Increment position (index is 0-based, progress is 1-based typically)
                        let new_pos = index + 1;
                        debug!(
                            "Callback: Setting commit pads progress bar position to {}",
                            new_pos
                        );
                        commit_pb.set_position(new_pos); 
                    } else {
                        debug!("Callback: Commit pads progress bar is already finished.");
                    }
                    // Don't finish here, wait for StoreComplete or error
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
        reservation_confirmed_and_needed,
        callback,
    )
}
