use crate::connect_to_daemon;
use anyhow::Result;

pub async fn handle_stats() -> Result<()> {
    let mut client = connect_to_daemon().await?;
    let stats = client.get_stats().await?;

    println!("Storage Statistics:");
    println!("-------------------");
    println!("Total Keys: {}", stats.total_keys);
    println!("Total Pads Managed:    {}", stats.total_pads);
    println!("  Occupied (Private):  {}", stats.occupied_pads);
    println!("  Free Pads:           {}", stats.free_pads);
    println!("  Pending Verify Pads: {}", stats.pending_verify_pads);

    Ok(())
}
