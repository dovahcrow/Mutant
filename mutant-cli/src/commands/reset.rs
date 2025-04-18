use log::{error, info};
use mutant_lib::MutAnt;
use std::process::ExitCode;

use dialoguer::Confirm;

pub async fn handle_reset(mutant: MutAnt) -> ExitCode {
    if !Confirm::new()
        .with_prompt(
            "WARNING: This will reset the index, orphaning all existing data. Are you sure?",
        )
        .interact()
        .unwrap_or(false)
    {
        println!("Reset cancelled.");
        return ExitCode::SUCCESS;
    }

    info!("Proceeding with index reset...");
    match mutant.reset().await {
        Ok(_) => {
            println!("Index reset successfully.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("Error during index reset: {}", e);
            eprintln!("Error resetting index: {}", e);
            ExitCode::FAILURE
        }
    }
}
