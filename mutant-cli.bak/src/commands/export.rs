use log::{error, info};
use mutant_lib::{MutAnt, error::Error as LibError, storage::PadInfo};
use std::{fs::File, io::BufWriter, process::ExitCode};

pub async fn handle_export(mutant: MutAnt, output: String) -> ExitCode {
    info!("CLI: Handling Export command");

    match mutant.export_raw_pads_private_key().await {
        Ok(pads_hex) => {
            write_pads_to_file(&output, pads_hex).await.unwrap();
            println!("Successfully exported free pad.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("Unexpected error during export: {}", e);
            eprintln!("An unexpected error occurred during export: {}", e);
            ExitCode::FAILURE
        }
    }
}

async fn write_pads_to_file(output: &str, pads_hex: Vec<PadInfo>) -> Result<(), LibError> {
    let file = File::create(output)
        .map_err(|e| LibError::Internal(format!("Failed to create file: {}", e)))?;
    let writer = BufWriter::new(file);
    serde_cbor::to_writer(writer, &pads_hex)
        .map_err(|e| LibError::Internal(format!("Failed to write pads: {}", e)))?;
    Ok(())
}
