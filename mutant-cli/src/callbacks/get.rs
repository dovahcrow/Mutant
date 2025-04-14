use super::progress::StyledProgressBar;
use mutant_lib::events::{GetCallback, GetEvent};
use futures::future::FutureExt;
use indicatif::MultiProgress;
use log::{debug, warn};
use std::sync::{Arc};
use tokio::sync::Mutex;

pub fn create_get_callback(
    multi_progress: &MultiProgress,
    quiet: bool,
) -> (Arc<Mutex<Option<StyledProgressBar>>>, GetCallback) {
    let download_pb_opt = Arc::new(Mutex::new(None::<StyledProgressBar>));

    if quiet {
        let noop_callback: GetCallback = Box::new(|_event: GetEvent| async move { Ok(()) }.boxed());
        return (download_pb_opt, noop_callback);
    }

    let pb_clone = download_pb_opt.clone();
    let mp_clone = multi_progress.clone();

    let callback: GetCallback = Box::new(move |event: GetEvent| {
        let pb_arc = pb_clone.clone();
        let multi_progress = mp_clone.clone();

        async move {
            let mut pb_guard = pb_arc.lock().await;

            match event {
                GetEvent::StartingDownload { total_bytes } => {
                    let pb = pb_guard.get_or_insert_with(|| {
                         let pb = StyledProgressBar::new(&multi_progress);
                         pb.set_style(super::progress::get_default_bytes_style());
                         pb.set_message("Downloading...".to_string());
                         pb
                    });
                    pb.set_length(total_bytes);
                    pb.set_position(0); 
                    debug!("Get Callback: StartingDownload - Set length to {}", total_bytes);
                }
                GetEvent::DownloadProgress { bytes_read, total_bytes: _ } => {
                    if let Some(pb) = pb_guard.as_mut() {
                        if !pb.is_finished() {
                            pb.set_position(bytes_read);
                        }
                    } else {
                        warn!("Get Callback: DownloadProgress event received but progress bar does not exist.");
                    }
                }
                GetEvent::DownloadFinished => {
                    if let Some(pb) = pb_guard.take() { 
                        if !pb.is_finished() {
                            pb.finish_and_clear();
                            debug!("Get Callback: DownloadFinished - Progress bar finished and cleared.");
                        }
                    } else {
                        debug!("Get Callback: DownloadFinished event received but progress bar was already finished or never existed.");
                    }
                }
            }
             Ok(()) 
        }.boxed() 
    });

    (download_pb_opt, callback)
}
