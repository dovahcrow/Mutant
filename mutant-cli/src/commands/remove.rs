use log::debug;

use mutant_lib::MutAnt;
use std::process::ExitCode;

pub async fn handle_rm(mutant: MutAnt, key: String) -> ExitCode {
    debug!("CLI: Handling Rm command: key={}", key);
    match mutant.remove(&key).await {
        Ok(_) => {
            println!("Removed key: {}", key);
            ExitCode::SUCCESS
        }

        Err(e) => {
            eprintln!("Error removing key: {}", e);
            ExitCode::FAILURE
        }
    }
}
