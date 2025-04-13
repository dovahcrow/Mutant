use crate::app::CliError;
use log::{debug, error, info, trace, warn};
use mutant_lib::cache::{read_local_index, write_local_index};
use mutant_lib::mutant::MutAnt;
use mutant_lib::mutant::data_structures::MasterIndexStorage;
use std::collections::HashSet;

pub async fn handle_sync(mutant: MutAnt) -> Result<(), CliError> {
    info!("Starting synchronization process...");

    // 1. Read local cache
    info!("Reading local index cache...");
    let local_index_opt = read_local_index().await.map_err(|e| {
        error!("Failed to read local cache during sync: {}", e);
        CliError::from(e) // Convert LibError to CliError
    })?;
    let local_index = local_index_opt.unwrap_or_else(|| {
        warn!("Local cache not found or empty, starting sync with default empty index.");
        MasterIndexStorage::default()
    });

    // 2. Fetch remote index
    info!("Fetching remote index...");
    let remote_index = mutant.fetch_remote_master_index().await.map_err(|e| {
        error!("Failed to fetch remote index during sync: {}", e);
        CliError::from(e)
    })?;

    // 3. Merge indices
    info!("Merging local and remote indices...");
    let mut merged_index = remote_index.clone(); // Start with remote as base
    let mut local_keys_added = 0;
    let mut remote_keys_found = 0;

    // Add local keys not present in remote
    for (key, local_info) in local_index.index.iter() {
        if !merged_index.index.contains_key(key) {
            debug!("Sync: Adding key '{}' from local to merged index.", key);
            merged_index.index.insert(key.clone(), local_info.clone());
            local_keys_added += 1;
        } else {
            remote_keys_found += 1;
            // TODO: Later, implement conflict resolution if needed (e.g., based on timestamp)
            trace!(
                "Sync: Key '{}' exists in both local and remote. Using remote version.",
                key
            );
        }
    }

    // Merge free pads (simple union, remove duplicates)
    let mut remote_pads_set: HashSet<_> = remote_index
        .free_pads
        .iter()
        .map(|(addr, _)| *addr)
        .collect();
    let mut local_pads_added = 0;
    for (addr, key_bytes) in local_index.free_pads.iter() {
        if remote_pads_set.insert(*addr) {
            // insert returns true if value was not present
            merged_index.free_pads.push((*addr, key_bytes.clone()));
            local_pads_added += 1;
            debug!("Sync: Adding free pad {} from local.", addr);
        }
    }
    info!(
        "Merged index: {} total keys ({} from local added), {} total free pads ({} from local added).",
        merged_index.index.len(),
        local_keys_added,
        merged_index.free_pads.len(),
        local_pads_added
    );

    // Ensure scratchpad size consistency (remote wins)
    if local_index.scratchpad_size != 0
        && local_index.scratchpad_size != remote_index.scratchpad_size
    {
        warn!(
            "Local scratchpad size ({}) differs from remote ({}). Using remote size.",
            local_index.scratchpad_size, remote_index.scratchpad_size
        );
    }
    // merged_index already has the remote size due to clone

    // 4. Update in-memory state (Need mutable access to MutAnt fields, which we don't have directly)
    // We need internal access to MutAnt's Arc<Mutex<MasterIndexStorage>> to update it.
    // Let's add an internal method to MutAnt for this.
    info!("Updating in-memory index...");
    mutant
        .update_internal_master_index(merged_index.clone())
        .await
        .map_err(CliError::from)?;

    // 5. Write local cache
    info!("Writing merged index to local cache...");
    write_local_index(&merged_index).await.map_err(|e| {
        error!("Failed to write merged index to local cache: {}", e);
        CliError::from(e)
    })?;

    // 6. Save remote index
    info!("Saving merged index to remote storage...");
    mutant.save_master_index().await.map_err(|e| {
        error!("Failed to save merged index to remote storage: {}", e);
        CliError::from(e)
    })?;

    // 7. Print status
    println!("Synchronization complete.");
    println!(
        "  {} keys added from local cache to remote.",
        local_keys_added
    );
    println!("  {} keys already existed remotely.", remote_keys_found);
    println!(
        "  {} free pads added from local cache to remote.",
        local_pads_added
    );
    println!("  Total keys: {}", merged_index.index.len());
    println!("  Total free pads: {}", merged_index.free_pads.len());

    Ok(())
}
