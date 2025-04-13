use log::debug;
use mutant_lib::{error::Error, mutant::MutAnt};
use std::process::ExitCode;

pub async fn handle_rm(mutant: MutAnt, key: String) -> ExitCode {
    debug!("CLI: Handling Rm command: key={}", key);
    match mutant.remove(&key).await {
        Ok(_) => {
            println!("Removed key: {}", key);
            ExitCode::SUCCESS
        }
        Err(Error::KeyNotFound(_)) => {
            eprintln!("Key not found: {}", key);
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("Error removing key: {}", e);
            ExitCode::FAILURE
        }
    }
}
