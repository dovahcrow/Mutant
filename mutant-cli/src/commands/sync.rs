use crate::app::CliError;
use crate::callbacks::progress::StyledProgressBar;
use indicatif::MultiProgress;
use log::{debug, error, info, trace, warn};
use mutant_lib::autonomi::ScratchpadAddress;
use mutant_lib::cache::{read_local_index, write_local_index};
use mutant_lib::mutant::MutAnt;
use mutant_lib::mutant::data_structures::MasterIndexStorage;
use std::collections::{HashMap, HashSet};

pub async fn handle_sync(mutant: MutAnt, push_force: bool) -> Result<(), CliError> {
    info!("Starting synchronization process...");
    let network = mutant.get_network_choice();
    let mp = MultiProgress::new();
    let pb = StyledProgressBar::new_for_steps(&mp);

    if push_force {
        let total_steps = 4;
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting push-force sync...".to_string());

        // --- Step 1: Read local cache ---
        pb.set_position(1);
        pb.set_message("Reading local index cache...".to_string());
        let local_index = read_local_index(network)
            .await
            .map_err(|e| {
                let msg = format!("Failed to read local cache: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::from(e)
            })?
            .ok_or_else(|| {
                let msg = "Local cache not found or empty. Cannot push-force.".to_string();
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::MutAntInit(msg)
            })?;

        // --- Step 2: Update in-memory state ---
        pb.set_position(2);
        pb.set_message("Updating in-memory index...".to_string());
        mutant
            .update_internal_master_index(local_index.clone())
            .await
            .map_err(|e| {
                let msg = format!("Failed to update in-memory index: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::from(e)
            })?;

        // --- Step 3: Write local cache ---
        pb.set_position(3);
        pb.set_message("Rewriting local index cache...".to_string());
        write_local_index(&local_index, network)
            .await
            .map_err(|e| {
                let msg = format!("Failed to rewrite local cache: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::from(e)
            })?;

        // --- Step 4: Save remote index (Force overwrite) ---
        pb.set_position(4);
        pb.set_message("Saving index to remote (force)...".to_string());
        mutant.save_master_index().await.map_err(|e| {
            let msg = format!("Failed to save index to remote: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;

        pb.finish_and_clear();
        println!("Push-force synchronization complete. Remote index overwritten with local cache.");
        println!("  Total keys: {}", local_index.index.len());
        println!("  Total free pads: {}", local_index.free_pads.len());

        Ok(())
    } else {
        // --- Regular Sync Logic ---
        let total_steps = 6;
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting regular sync...".to_string());

        // --- Step 1: Read local cache ---
        pb.set_position(1);
        pb.set_message("Reading local index cache...".to_string());
        let local_index_opt = read_local_index(network).await.map_err(|e| {
            let msg = format!("Failed to read local cache: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;
        let local_index = local_index_opt.unwrap_or_else(|| {
            warn!("Local cache not found or empty, starting sync with default empty index.");
            MasterIndexStorage::default()
        });

        // --- Step 2: Fetch remote index (or create if not found) ---
        pb.set_position(2);
        pb.set_message("Fetching remote index...".to_string());
        let remote_index = match mutant.fetch_remote_master_index().await {
            Ok(index) => {
                info!("Successfully fetched remote index.");
                index
            }
            Err(mutant_lib::error::Error::MasterIndexNotFound) => {
                warn!("Remote master index not found. Creating remote index from current state.");
                pb.set_message("Remote index not found, creating...".to_string());
                if let Err(e) = mutant.save_master_index().await {
                    let msg = format!("Failed to create remote index: {}", e);
                    error!("{}", msg);
                    pb.abandon_with_message(msg.clone());
                    return Err(CliError::from(e));
                }
                info!("Successfully created remote index.");
                pb.set_message("Fetching newly created index...".to_string());
                // Fetch it again to use for the merge
                match mutant.fetch_remote_master_index().await {
                    Ok(newly_created_index) => newly_created_index,
                    Err(e) => {
                        let msg = format!("Failed to fetch newly created remote index: {}", e);
                        error!("{}", msg);
                        pb.abandon_with_message(msg.clone());
                        return Err(CliError::from(e));
                    }
                }
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
        pb.set_message("Merging local and remote indices...".to_string());

        let mut merged_index = remote_index.clone();
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

        pb.finish_and_clear();
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
