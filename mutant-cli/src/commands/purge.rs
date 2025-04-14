use clap::Args;
use log::info;
use mutant_lib::MutAnt;
use std::error::Error;

#[derive(Args, Debug)]
pub struct PurgeArgs {}

pub async fn run(_args: PurgeArgs, mutant: MutAnt) -> Result<(), Box<dyn Error>> {
    info!("Executing purge command...");

    mutant
        .purge_unverified_pads()
        .await
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;

    info!("Purge command finished successfully.");

    Ok(())
}
