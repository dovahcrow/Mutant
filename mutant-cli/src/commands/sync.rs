use crate::app::CliError;
use log::{debug, error, info, trace, warn};
use mutant_lib::autonomi::ScratchpadAddress;
use mutant_lib::cache::{read_local_index, write_local_index};
use mutant_lib::mutant::MutAnt;
use mutant_lib::mutant::data_structures::MasterIndexStorage;
use std::collections::{HashMap, HashSet};

pub async fn handle_sync(mutant: MutAnt, push_force: bool) -> Result<(), CliError> {
    info!("Starting synchronization process...");
    let network = mutant.get_network_choice();

    if push_force {
        info!("Push-force sync initiated: Overwriting remote index with local cache.");

        // 1. Read local cache
        info!("Reading local index cache...");
        let local_index = read_local_index(network)
            .await
            .map_err(|e| {
                error!("Failed to read local cache for push-force sync: {}", e);
                CliError::from(e)
            })?
            .ok_or_else(|| {
                error!("Local cache not found or empty. Cannot push-force an empty index.");
                // Consider a more specific error type if needed
                CliError::MutAntInit("Local cache is empty, cannot push-force.".to_string())
            })?;

        // 2. Update in-memory state
        info!("Updating in-memory index with local cache...");
        mutant
            .update_internal_master_index(local_index.clone())
            .await
            .map_err(CliError::from)?;

        // 3. Write local cache (optional, but good for consistency)
        info!("Rewriting local index cache...");
        write_local_index(&local_index, network)
            .await
            .map_err(|e| {
                error!(
                    "Failed to rewrite local cache during push-force sync: {}",
                    e
                );
                CliError::from(e)
            })?;

        // 4. Save remote index (Force overwrite)
        info!("Saving local index to remote storage (force push)...");
        // Assuming save_master_index handles the overwrite implicitly
        mutant.save_master_index().await.map_err(|e| {
            error!(
                "Failed to save index to remote storage during push-force sync: {}",
                e
            );
            CliError::from(e)
        })?;

        println!("Push-force synchronization complete. Remote index overwritten with local cache.");
        println!("  Total keys: {}", local_index.index.len());
        println!("  Total free pads: {}", local_index.free_pads.len());

        Ok(())
    } else {
        // --- Regular Sync Logic ---
        info!("Performing regular merge sync...");

        // 1. Read local cache
        info!("Reading local index cache...");
        let local_index_opt = read_local_index(network).await.map_err(|e| {
            error!("Failed to read local cache during sync: {}", e);
            CliError::from(e) // Convert LibError to CliError
        })?;
        let local_index = local_index_opt.unwrap_or_else(|| {
            warn!("Local cache not found or empty, starting sync with default empty index.");
            MasterIndexStorage::default()
        });

        // 2. Fetch remote index
        info!("Fetching remote index...");
        let remote_index = match mutant.fetch_remote_master_index().await {
            Ok(index) => {
                info!("Successfully fetched remote index.");
                index
            }
            Err(mutant_lib::error::Error::MasterIndexNotFound) => {
                warn!("Remote master index not found. Creating remote index from current state.");
                // The MutAnt instance should already hold the correct default index
                // (including size) from initialization.
                // Save the current state to create the remote index.
                if let Err(e) = mutant.save_master_index().await {
                    error!(
                        "Failed to create default remote index from current state during sync: {}",
                        e
                    );
                    return Err(CliError::from(e));
                }
                info!("Successfully created remote index from current state.");

                // Now that it's created, fetch it again to use for the merge.
                match mutant.fetch_remote_master_index().await {
                    Ok(newly_created_index) => newly_created_index,
                    Err(e) => {
                        error!(
                            "Failed to fetch the newly created remote index during sync: {}",
                            e
                        );
                        return Err(CliError::from(e));
                    }
                }
            }
            Err(e) => {
                // Handle other potential errors during the initial fetch
                error!("Failed to fetch remote index during sync: {}", e);
                return Err(CliError::from(e));
            }
        };

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

        // --- Corrected Free Pad Merging Logic ---
        // 1. Collect all occupied pad addresses from the merged index
        let occupied_pads: HashSet<ScratchpadAddress> = merged_index
            .index
            .values()
            .flat_map(|key_info| key_info.pads.iter().map(|pad_info| pad_info.address))
            .collect();
        debug!(
            "Sync: Found {} occupied pads in merged index.",
            occupied_pads.len()
        );

        // 2. Create a combined map of potential free pads (local + remote), prioritizing remote keys if duplicates exist
        //    Using a map ensures duplicate addresses are handled (last write wins, so remote wins).
        let mut potential_free_pads_map: HashMap<ScratchpadAddress, Vec<u8>> = HashMap::new();
        // Add local free pads first
        potential_free_pads_map.extend(local_index.free_pads.iter().cloned());
        // Add remote free pads, overwriting local entries if addresses conflict (remote is source of truth for keys)
        potential_free_pads_map.extend(remote_index.free_pads.iter().cloned());

        let combined_candidates_count = potential_free_pads_map.len(); // Get count before move

        // 3. Filter the potential free pads, keeping only those not occupied
        let final_free_pads: Vec<(ScratchpadAddress, Vec<u8>)> = potential_free_pads_map
            .into_iter()
            .filter(|(addr, _)| !occupied_pads.contains(addr))
            .collect();

        let original_remote_free_count = remote_index.free_pads.len();
        let final_free_count = final_free_pads.len();
        let local_pads_actually_occupied = local_index
            .free_pads
            .iter()
            .filter(|(addr, _)| occupied_pads.contains(addr))
            .count();

        debug!(
            "Sync: Merged free pads. Started with {} remote, {} combined candidates. Filtered out {} occupied. Final free count: {}.",
            original_remote_free_count,
            combined_candidates_count, // Use the stored count
            combined_candidates_count - final_free_count,
            final_free_count
        );

        // Update the merged index's free pad list
        merged_index.free_pads = final_free_pads;

        // Recalculate local_pads_added based on the final list compared to the initial remote list's addresses
        let remote_pads_addr_set: HashSet<_> = remote_index
            .free_pads
            .iter()
            .map(|(addr, _)| *addr)
            .collect();
        let local_pads_added = merged_index
            .free_pads
            .iter()
            .filter(|(addr, _)| !remote_pads_addr_set.contains(addr))
            .count();

        info!(
            "Merged index: {} total keys ({} from local added), {} final free pads ({} added vs remote, {} local pads were actually occupied/removed).",
            merged_index.index.len(),
            local_keys_added,
            final_free_count,
            local_pads_added,
            local_pads_actually_occupied
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
        write_local_index(&merged_index, network)
            .await
            .map_err(|e| {
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
}
