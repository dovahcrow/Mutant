use crate::callbacks::progress::StyledProgressBar;
use dialoguer::Confirm;
use indicatif::MultiProgress;
use mutant_lib::error::Error as LibError;
use mutant_lib::events::{InitCallback, InitProgressEvent};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

pub fn create_init_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<TokioMutex<Option<StyledProgressBar>>>, InitCallback) {
    let init_pb_opt = Arc::new(TokioMutex::new(None::<StyledProgressBar>));

    let pb_clone = init_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let init_callback: InitCallback = Box::new(move |event: InitProgressEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();
        let quiet_captured = quiet;

        Box::pin(async move {
            if quiet_captured {
                match event {
                    InitProgressEvent::PromptCreateRemoteIndex => {
                        let confirmation = Confirm::new()
                            .with_prompt("No local cache found, and no remote index exists. Create remote index now ? If not, you can sync it later.")
                            .interact()
                            
                            .map_err(|e| LibError::Internal(format!("Dialoguer interaction failed: {}", e)))
                        ?;
                        Ok(Some(confirmation))
                    }
                    _ => Ok(None),
                }
            } else {
                let mut pb_guard = pb_arc.lock().await;

                match event {
                    InitProgressEvent::Starting { total_steps } => {
                        let pb = pb_guard.get_or_insert_with(|| StyledProgressBar::new_for_steps(&multi_progress));
                        pb.set_length(total_steps);
                        pb.set_position(0);
                        pb.set_message("Initializing...".to_string());
                        Ok(None)
                    }
                    InitProgressEvent::Step { step, message } => {
                        if let Some(pb) = pb_guard.as_mut() {
                            if step > pb.position() {
                                pb.set_position(step);
                            }
                            pb.set_message(message.clone());
                        } else {
                            let pb = pb_guard.get_or_insert_with(|| {
                                let pb = StyledProgressBar::new_for_steps(&multi_progress);
                                pb.set_message(message.clone());
                                pb
                            });
                            pb.set_position(step);
                        }
                        Ok(None)
                    }
                    InitProgressEvent::Complete { message: _ } => {
                        if let Some(pb) = pb_guard.take() {
                            pb.finish_and_clear();
                        }
                        Ok(None)
                    }
                    InitProgressEvent::Failed { error_msg } => {
                        if let Some(pb) = pb_guard.take() {
                            pb.abandon_with_message(format!(
                                "Initialization Failed: {}",
                                error_msg
                            ));
                        }
                        Err(LibError::Internal(format!(
                            "Initialization failed: {}",
                            error_msg
                        )))
                    }
                    InitProgressEvent::PromptCreateRemoteIndex => {
                        drop(pb_guard);
                        let confirmation = multi_progress.suspend(|| {
                            Confirm::new()
                                .with_prompt("No local cache found, and no remote index exists. Create remote index now ? If not, you can sync it later.")
                                .interact()
                                
                                .map_err(|e| LibError::Internal(format!("Dialoguer interaction failed: {}", e)))
                        })?;
                        Ok(Some(confirmation))
                    }
                }
            }
        })
    });

    (init_pb_opt, init_callback)
}
