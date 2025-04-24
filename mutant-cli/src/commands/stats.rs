use log::{debug, info};

use mutant_lib::MutAnt;
use std::process::ExitCode;

pub async fn handle_stats(mutant: MutAnt) -> ExitCode {
    debug!("CLI: Handling Stats command");
    info!("Fetching stats...");
    let stats = mutant.get_storage_stats().await;
    println!("Storage Statistics:");
    println!("-------------------");

    println!("Total Keys: {}", stats.nb_keys);
    println!("Total Pads Managed:    {}", stats.total_pads);
    println!("  Occupied (Private):  {}", stats.occupied_pads);
    println!("  Free Pads:           {}", stats.free_pads);
    println!("  Pending Verify Pads: {}", stats.pending_verification_pads);
    ExitCode::SUCCESS
}
