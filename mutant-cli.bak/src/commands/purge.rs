use indicatif::MultiProgress;
use log::info;
use mutant_lib::MutAnt;
use mutant_lib::error::Error as LibError;
use std::error::Error as StdError;

use crate::callbacks::purge::create_purge_callback;

pub async fn run(
    aggressive: bool,
    mut mutant: MutAnt,
    mp: &MultiProgress,
    quiet: bool,
) -> Result<(), Box<dyn StdError>> {
    info!("Executing purge command...");

    let (_download_pb_opt, callback) = create_purge_callback(mp, quiet);

    mutant.set_purge_callback(callback).await;

    match mutant.purge(aggressive).await {
        Ok(_) => Ok(()),
        Err(e) => {
            if matches!(e, LibError::OperationCancelled) {
                info!("Purge operation cancelled.");
                Ok(())
            } else {
                Err(Box::new(e) as Box<dyn StdError>)
            }
        }
    }
}
