use super::progress::StyledProgressBar;
use anthill_lib::error::Error;
use anthill_lib::events::{PutCallback, PutEvent};
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
    sp_up_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    cf_pd_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>,
    res_confirmed_and_needed: Arc<Mutex<bool>>,
    last_displayed_reservation_progress: Arc<Mutex<u64>>,
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
    // Scratchpad upload progress bar
    Arc<Mutex<Option<StyledProgressBar>>>,
    // Flag indicating if reservation was confirmed and needed
    Arc<Mutex<bool>>,
    // Last displayed reservation progress value (internal detail)
    Arc<Mutex<u64>>,
    // The actual callback closure
    PutCallback,
) {
    let reservation_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let scratchpad_upload_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let confirmed_pads_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));
    let reservation_confirmed_and_needed = Arc::new(Mutex::new(false));
    let last_displayed_reservation_progress = Arc::new(Mutex::new(0u64));
    let total_bytes_for_upload = Arc::new(Mutex::new(0u64));

    // Create the context instance
    let context = PutCallbackContext {
        res_pb_opt: reservation_pb_opt.clone(),
        sp_up_pb_opt: scratchpad_upload_pb_opt.clone(),
        cf_pd_pb_opt: confirmed_pads_pb_opt.clone(),
        res_confirmed_and_needed: reservation_confirmed_and_needed.clone(),
        last_displayed_reservation_progress: last_displayed_reservation_progress.clone(),
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
                        let mut last_progress_guard = ctx.last_displayed_reservation_progress.lock().unwrap();
                        *last_progress_guard = 0;

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

                    let mut sp_up_pb_guard = ctx.sp_up_pb_opt.lock().unwrap();
                    let sp_up_pb = sp_up_pb_guard.get_or_insert_with(|| {
                        StyledProgressBar::new(&ctx.multi_progress)
                    });

                    sp_up_pb.set_style(super::progress::get_default_bytes_style());
                    sp_up_pb.set_length(total_bytes);
                    sp_up_pb.set_position(0);
                    sp_up_pb.set_message("Uploading...".to_string());

                    Ok(true)
                }
                PutEvent::UploadProgress { bytes_written, total_bytes: _ } => {
                    if let Some(sp_up_pb) = ctx.sp_up_pb_opt.lock().unwrap().as_mut() {
                        if !sp_up_pb.is_finished() {
                            sp_up_pb.set_position(bytes_written);
                        }
                    } else {
                        warn!("UploadProgress event received but upload progress bar does not exist.");
                    }
                    Ok(true)
                }
                PutEvent::UploadFinished => Ok(true),
                PutEvent::StoreComplete => {
                    if let Some(sp_up_pb) = ctx.sp_up_pb_opt.lock().unwrap().take() {
                        sp_up_pb.finish_and_clear();
                    }
                    if let Some(cf_pd_pb) = ctx.cf_pd_pb_opt.lock().unwrap().take() {
                        cf_pd_pb.finish_and_clear();
                    }
                    Ok(true)
                }
                PutEvent::ScratchpadUploadComplete { index: _, total: _ } => {
                    debug!("Received ScratchpadUploadComplete (ignored in CLI)");
                    Ok(true)
                }
                PutEvent::ScratchpadCommitComplete { index, total } => {
                    let mut cf_pd_pb_guard = ctx.cf_pd_pb_opt.lock().unwrap();
                    let cf_pd_pb = cf_pd_pb_guard.get_or_insert_with(|| {
                        debug!("Initializing confirmed pads progress bar ({} total)", total);
                        let pb = StyledProgressBar::new_for_steps(&ctx.multi_progress);
                        pb.set_length(total);
                        pb.set_message("Confirming pads...".to_string());
                        pb.reset();
                        pb
                    });

                    if !cf_pd_pb.is_finished() {
                        cf_pd_pb.set_position(index);
                    }
                    Ok(true)
                }
            }
        }
        .boxed()
    });

    (
        reservation_pb_opt,
        scratchpad_upload_pb_opt,
        reservation_confirmed_and_needed,
        last_displayed_reservation_progress,
        callback,
    )
}
