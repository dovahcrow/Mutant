use super::progress::StyledProgressBar;
use dialoguer::Confirm;
use futures::future::FutureExt;
use indicatif::MultiProgress;
use mutant_lib::error::Error;
use mutant_lib::events::{InitCallback, InitProgressEvent};
use std::sync::{Arc, Mutex};

pub fn create_init_callback(
    multi_progress: &MultiProgress,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, InitCallback) {
    let init_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    let pb_clone = init_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let init_callback: InitCallback = Arc::new(move |event: InitProgressEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        async move {
            let mut pb_guard = pb_arc.lock().unwrap();

            match event {
                InitProgressEvent::Starting { total_steps } => {
                    let pb = pb_guard.get_or_insert_with(|| {
                        let pb = StyledProgressBar::new_for_steps(&multi_progress);
                        pb
                    });
                    pb.set_length(total_steps);
                    pb.set_position(0);
                    pb.set_message("Initializing...".to_string());
                    Ok(None) // No decision needed
                }
                InitProgressEvent::Step { step, message } => {
                    if let Some(pb) = pb_guard.as_mut() {
                        if step > pb.position() {
                            pb.set_position(step);
                        }
                        pb.set_message(message.clone());
                    } else {
                        // Initialize if Step is the first event received (unlikely but possible)
                        let pb = pb_guard.get_or_insert_with(|| {
                            let pb = StyledProgressBar::new_for_steps(&multi_progress);
                            pb.set_message(message.clone());
                            pb
                        });
                        pb.set_position(step);
                    }
                    Ok(None) // No decision needed
                }
                InitProgressEvent::Complete { message: _ } => {
                    if let Some(pb) = pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    Ok(None) // No decision needed
                }
                InitProgressEvent::Failed { error_msg } => {
                    if let Some(pb) = pb_guard.take() {
                        pb.abandon_with_message(format!("Initialization Failed: {}", error_msg));
                    }
                    Ok(None) // No decision needed, failure is handled by the library return value
                }
                InitProgressEvent::ExistingLoaded { message: _ } => {
                    if let Some(pb) = pb_guard.take() {
                        pb.finish_and_clear();
                    }
                    Ok(None) // No decision needed
                }
                InitProgressEvent::PromptCreateRemoteIndex => {
                    // Important: Drop the progress bar lock *before* blocking on Confirm
                    drop(pb_guard);

                    // Suspend the MultiProgress to prevent it from overwriting the prompt
                    let confirmation = multi_progress.suspend(|| {
                         Confirm::new()
                            .with_prompt("No local cache found, and no remote index exists. Create remote index now?")
                            .interact()
                            .map_err(|e| mutant_lib::error::Error::DialoguerError(format!("Dialoguer error: {}", e)))
                    })?;

                    Ok(Some(confirmation))
                }
            }
        }
        .boxed()
    });

    (init_pb_opt, init_callback)
}
