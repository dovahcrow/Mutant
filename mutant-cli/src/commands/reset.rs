use log::{error, info};
// Use the new top-level re-exports
use dialoguer::Confirm;
use mutant_lib::MutAnt;
use std::process::ExitCode;

/// Executes the core logic for resetting the master index.
/// Confirmation should be handled by the caller.
pub async fn handle_reset(mutant: MutAnt) -> ExitCode {
    if !Confirm::new()
        .with_prompt(
            "WARNING: This will reset the index, orphaning all existing data. Are you sure?",
        )
        .interact()
        .unwrap_or(false)
    {
        println!("Reset cancelled.");
        return ExitCode::SUCCESS; // Or a specific code for cancellation?
    }

    info!("Proceeding with index reset...");
    match mutant.reset().await {
        // Call the public reset method
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
