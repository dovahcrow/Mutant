use log::{error, info};
use mutant_lib::{error::Error, mutant::MutAnt};
use std::process::ExitCode;

pub async fn handle_import(mutant: MutAnt, private_key: String) -> ExitCode {
    info!(
        "CLI: Handling Import command for key starting with: {}",
        &private_key[..std::cmp::min(private_key.len(), 8)]
    );
    match mutant.import_free_pad(&private_key).await {
        Ok(_) => {
            println!("Successfully imported free pad.");
            ExitCode::SUCCESS
        }
        Err(Error::PadAlreadyExists(msg)) => {
            eprintln!("Import failed: {}", msg);
            ExitCode::FAILURE
        }
        Err(Error::InvalidInput(msg)) => {
            eprintln!("Import failed due to invalid private key: {}", msg);
            ExitCode::FAILURE
        }
        Err(e) => {
            error!("Unexpected error during import: {}", e);
            eprintln!("An unexpected error occurred during import: {}", e);
            ExitCode::FAILURE
        }
    }
}
