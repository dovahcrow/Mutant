use super::progress::StyledProgressBar;
use futures::future::FutureExt;
use indicatif::MultiProgress;
use mutant_lib::events::{InitCallback, InitProgressEvent};

pub fn create_init_callback(multi_progress: &MultiProgress) -> (StyledProgressBar, InitCallback) {
    let init_pb = StyledProgressBar::new_for_steps(multi_progress);
    let init_pb_clone = init_pb.clone();

    let init_callback: InitCallback = Box::new(move |event: InitProgressEvent| {
        let pb = init_pb_clone.clone();
        async move {
            match event {
                InitProgressEvent::Starting { total_steps } => {
                    pb.set_length(total_steps);
                    pb.set_position(0);
                    pb.set_message("Initializing...".to_string());
                }
                InitProgressEvent::Step { step, message } => {
                    if step > pb.position() {
                        pb.set_position(step);
                    }
                    pb.set_message(message.clone());
                }
                InitProgressEvent::Complete { message } => {
                    pb.set_message(message);
                    pb.finish_and_clear();
                }
                InitProgressEvent::Failed { error_msg } => {
                    pb.abandon_with_message(format!("Initialization Failed: {}", error_msg));
                }
                InitProgressEvent::ExistingLoaded { message } => {
                    pb.set_message(message);
                    pb.finish_and_clear();
                }
            }
            Ok(())
        }
        .boxed()
    });

    (init_pb, init_callback)
}
