use crate::app::CliError;
use crate::callbacks::sync::create_sync_callback;
use indicatif::MultiProgress;
use log::{error, info};
use mutant_lib::MutAnt;

pub async fn run(
    mut mutant: MutAnt,
    push_force: bool,
    multi_progress: &MultiProgress,
    quiet: bool,
) -> Result<(), CliError> {
    info!("Starting synchronization process...");

    let (_download_pb_opt, callback) = create_sync_callback(multi_progress, quiet);

    mutant.set_sync_callback(callback).await;

    match mutant.sync(push_force).await {
        Ok(result) => {
            println!("Synchronization complete.");
            println!("  {} keys added", result.nb_keys_added);
            println!("  {} keys updated", result.nb_keys_updated);
            println!("  {} free pads added", result.nb_free_pads_added);
            println!("  {} pending pads added", result.nb_pending_pads_added);
            return Ok(());
        }
        Err(e) => {
            error!("Sync command failed: {}", e);
            return Err(CliError::from(e));
        }
    }
}
