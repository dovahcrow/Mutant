use log::{error, info};
use mutant_lib::{MutAnt, error::Error as LibError, storage::PadInfo};
use std::{fs::File, io::BufReader, process::ExitCode};

pub async fn handle_import(mut mutant: MutAnt, input: String) -> ExitCode {
    info!("CLI: Handling Import command");

    let pads_hex = read_pads_from_file(&input).await.unwrap();

    match mutant.import_raw_pads_private_key(pads_hex).await {
        Ok(_) => {
            println!("Successfully imported free pad.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("Unexpected error during import: {}", e);
            eprintln!("An unexpected error occurred during import: {}", e);
            ExitCode::FAILURE
        }
    }
}

async fn read_pads_from_file(input: &str) -> Result<Vec<PadInfo>, LibError> {
    let file =
        File::open(input).map_err(|e| LibError::Internal(format!("Failed to open file: {}", e)))?;
    let reader = BufReader::new(file);
    let pads: Vec<PadInfo> = serde_cbor::from_reader(reader)
        .map_err(|e| LibError::Internal(format!("Failed to deserialize pads: {}", e)))?;
    Ok(pads)
}
