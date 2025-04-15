use log::debug;
// Use new top-level re-exports
use mutant_lib::MutAnt;
use std::process::ExitCode;

pub async fn handle_rm(mutant: MutAnt, key: String) -> ExitCode {
    debug!("CLI: Handling Rm command: key={}", key);
    match mutant.remove(&key).await {
        Ok(_) => {
            println!("Removed key: {}", key);
            ExitCode::SUCCESS
        }
        // Update error matching. Note: remove currently returns Ok even if key not found.
        // If KeyNotFound needs specific handling, the LibError::Data(DataError::KeyNotFound)
        // variant would be used, but the current lib implementation doesn't return it on remove.
        // We'll keep the generic error handling for now.
        Err(e) => {
            eprintln!("Error removing key: {}", e);
            ExitCode::FAILURE
        }
    }
}
