use crate::app::CliError;
use crate::callbacks::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{debug, error, info, trace, warn};
// Use new top-level re-exports and correct paths
use mutant_lib::{
    Error as LibError,
    MutAnt,
    autonomi::ScratchpadAddress,      // Use re-exported autonomi crate
    index::{IndexError, MasterIndex}, // Import from index module
};
// Removed direct import of autonomi::ScratchpadAddress
use std::collections::{HashMap, HashSet};

pub async fn handle_sync(mutant: MutAnt, push_force: bool) -> Result<(), CliError> {
    info!("Starting synchronization process...");
    let _network = mutant.get_network_choice(); // Keep for potential future use
    let mp = MultiProgress::new();
    let pb = StyledProgressBar::new_for_steps(&mp);

    if push_force {
        // --- Push Force Logic ---
        // Force save the current in-memory index state to remote storage.
        let total_steps = 1; // Only 1 step: Save Remote
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting push-force sync...".to_string());

        // --- Step 1: Save remote index (Force overwrite) ---
        pb.set_position(1);
        pb.set_message("Saving current index to remote (force)...".to_string());
        mutant.save_master_index().await.map_err(|e| {
            let msg = format!("Failed to save index to remote: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e) // Convert LibError to CliError
        })?;

        pb.finish_with_message(
            "Push-force sync complete. Remote index overwritten with current in-memory state.",
        );

        Ok(())
    } else {
        // --- Regular Sync Logic ---
        let total_steps = 5; // 1: Get Local, 2: Fetch Remote, 3: Merge, 4: Update Local+Remote, 5: Update Cache
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting regular sync...".to_string());

        // --- Step 1: Get current in-memory index (represents 'local' state for merge) ---
        pb.set_position(1);
        pb.set_message("Getting current in-memory index...".to_string());
        let local_index = mutant.get_index_copy().await.map_err(|e| {
            let msg = format!("Failed to get current index state: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e) // Convert LibError to CliError
        })?;

        // --- Step 2: Fetch remote index (or create if not found) ---
        pb.set_position(2);
        pb.set_message("Fetching remote index...".to_string());
        let remote_index = match mutant.fetch_remote_master_index().await {
            Ok(index) => {
                info!("Successfully fetched remote index.");
                index
            }
            Err(LibError::Index(IndexError::KeyNotFound(_))) => {
                warn!("Remote master index not found. Saving current in-memory state as remote.");
                pb.set_message("Remote index not found, creating...".to_string());
                // Save the current in-memory state (which might be default or from cache)
                if let Err(e) = mutant.save_master_index().await {
                    let msg = format!("Failed to create remote index: {}", e);
                    error!("{}", msg);
                    pb.abandon_with_message(msg.clone());
                    return Err(CliError::from(e));
                }
                info!("Successfully created remote index from in-memory state.");
                // Use the state we just saved as the 'remote' for merging (it's now the source of truth)
                local_index.clone() // Use the in-memory state we just pushed
            }
            Err(e) => {
                let msg = format!("Failed to fetch remote index: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                return Err(CliError::from(e));
            }
        };

        // --- Step 3: Merge indices ---
        pb.set_position(3);
        pb.set_message("Merging in-memory and remote indices...".to_string());

        let mut merged_index = remote_index.clone(); // Start with remote as base
        let mut local_keys_added = 0;
        let mut remote_keys_found = 0;

        // Merge keys: Add local keys not present in remote
        for (key, local_info) in local_index.index.iter() {
            if !merged_index.index.contains_key(key) {
                debug!("Sync: Adding key '{}' from local to merged index.", key);
                merged_index.index.insert(key.clone(), local_info.clone());
                local_keys_added += 1;
            } else {
                remote_keys_found += 1;
                // Conflict resolution (remote wins for now)
                trace!("Sync: Key '{}' exists in both. Using remote version.", key);
            }
        }

        // Merge free pads: Combine unique pads, prioritizing remote keys
        let occupied_pads: HashSet<ScratchpadAddress> = merged_index
            .index
            .values()
            .flat_map(|key_info| key_info.pads.iter().map(|pad_info| pad_info.address))
            .collect();

        let mut potential_free_pads_map: HashMap<ScratchpadAddress, Vec<u8>> = HashMap::new();
        potential_free_pads_map.extend(local_index.free_pads.iter().cloned());
        potential_free_pads_map.extend(remote_index.free_pads.iter().cloned()); // Remote overwrites duplicates

        let final_free_pads: Vec<(ScratchpadAddress, Vec<u8>)> = potential_free_pads_map
            .into_iter()
            .filter(|(addr, _)| !occupied_pads.contains(addr))
            .collect();

        let remote_pads_addr_set: HashSet<_> = remote_index
            .free_pads
            .iter()
            .map(|(addr, _)| *addr)
            .collect();
        let local_pads_added = final_free_pads
            .iter()
            .filter(|(addr, _)| !remote_pads_addr_set.contains(addr))
            .count();

        merged_index.free_pads = final_free_pads;

        // Ensure scratchpad size consistency (remote wins)
        if local_index.scratchpad_size != 0
            && local_index.scratchpad_size != remote_index.scratchpad_size
        {
            warn!(
                "Local scratchpad size ({}) differs from remote ({}). Using remote size.",
                local_index.scratchpad_size, remote_index.scratchpad_size
            );
        }
        merged_index.scratchpad_size = remote_index.scratchpad_size; // Ensure remote size is used

        info!(
            "Merged index: {} total keys ({} from local added), {} final free pads ({} added vs remote).",
            merged_index.index.len(),
            local_keys_added,
            merged_index.free_pads.len(),
            local_pads_added
        );

        // --- Step 4: Update In-Memory and Save Remote ---
        pb.set_position(4);
        pb.set_message("Updating state and saving remote index...".to_string());
        // Update in-memory state first
        mutant
            .update_internal_master_index(merged_index.clone())
            .await
            .map_err(|e| {
                let msg = format!("Failed to update in-memory index: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::from(e)
            })?;
        // Now save the updated state (which is now the merged state) remotely
        mutant.save_master_index().await.map_err(|e| {
            let msg = format!("Failed to save merged index to remote: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;

        // --- Step 5: Update Local Cache ---
        pb.set_position(5);
        pb.set_message("Updating local cache...".to_string());
        // Don't fail the whole sync for a cache write error, just warn.
        if let Err(e) = mutant.save_index_cache().await {
            warn!("Failed to update local cache after sync: {}", e);
        }

        pb.finish_with_message("Synchronization complete.");
        println!("Synchronization complete.");
        println!("  {} keys added from local to remote.", local_keys_added);
        println!("  {} keys already existed remotely.", remote_keys_found);
        println!(
            "  {} free pads added from local to remote.",
            local_pads_added
        );
        println!("  Total keys: {}", merged_index.index.len());
        println!("  Total free pads: {}", merged_index.free_pads.len());

        Ok(())
    }
}
