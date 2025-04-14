use crate::app::CliError;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{debug, error, info, trace, warn};
use mutant_lib::autonomi::ScratchpadAddress;
use mutant_lib::cache::{read_local_index, write_local_index};
use mutant_lib::mutant::MutAnt;
use mutant_lib::mutant::data_structures::MasterIndexStorage;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

pub async fn handle_sync(mutant: MutAnt, push_force: bool) -> Result<(), CliError> {
    info!("Starting synchronization process...");
    let network = mutant.get_network_choice();
    let mp = MultiProgress::new();
    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{prefix:.bold.dim} {spinner} {wide_msg}")
        .expect("Invalid progress style template");

    if push_force {
        let pb_read = mp.add(ProgressBar::new_spinner());
        pb_read.set_style(spinner_style.clone());
        pb_read.set_prefix("1/4");
        pb_read.set_message("Reading local index cache...");
        pb_read.enable_steady_tick(Duration::from_millis(100));

        let local_index = read_local_index(network)
            .await
            .map_err(|e| {
                error!("Failed to read local cache for push-force sync: {}", e);
                pb_read.finish_with_message("Failed to read local cache.");
                CliError::from(e)
            })?
            .ok_or_else(|| {
                error!("Local cache not found or empty. Cannot push-force an empty index.");
                pb_read.finish_with_message("Local cache empty.");
                CliError::MutAntInit("Local cache is empty, cannot push-force.".to_string())
            })?;
        pb_read.finish_with_message("Read local index cache.");

        let pb_update_mem = mp.add(ProgressBar::new_spinner());
        pb_update_mem.set_style(spinner_style.clone());
        pb_update_mem.set_prefix("2/4");
        pb_update_mem.set_message("Updating in-memory index...");
        pb_update_mem.enable_steady_tick(Duration::from_millis(100));
        mutant
            .update_internal_master_index(local_index.clone())
            .await
            .map_err(|e| {
                pb_update_mem.finish_with_message("Failed to update in-memory index.");
                CliError::from(e)
            })?;
        pb_update_mem.finish_with_message("Updated in-memory index.");

        let pb_write_cache = mp.add(ProgressBar::new_spinner());
        pb_write_cache.set_style(spinner_style.clone());
        pb_write_cache.set_prefix("3/4");
        pb_write_cache.set_message("Rewriting local index cache...");
        pb_write_cache.enable_steady_tick(Duration::from_millis(100));
        write_local_index(&local_index, network)
            .await
            .map_err(|e| {
                error!(
                    "Failed to rewrite local cache during push-force sync: {}",
                    e
                );
                pb_write_cache.finish_with_message("Failed to rewrite local cache.");
                CliError::from(e)
            })?;
        pb_write_cache.finish_with_message("Rewrote local index cache.");

        let pb_save_remote = mp.add(ProgressBar::new_spinner());
        pb_save_remote.set_style(spinner_style.clone());
        pb_save_remote.set_prefix("4/4");
        pb_save_remote.set_message("Saving index to remote (force)...");
        pb_save_remote.enable_steady_tick(Duration::from_millis(100));
        mutant.save_master_index().await.map_err(|e| {
            error!(
                "Failed to save index to remote storage during push-force sync: {}",
                e
            );
            pb_save_remote.finish_with_message("Failed to save remote index.");
            CliError::from(e)
        })?;
        pb_save_remote.finish_with_message("Saved index to remote.");

        mp.clear().expect("Failed to clear multi progress bar");
        println!("Push-force synchronization complete. Remote index overwritten with local cache.");
        println!("  Total keys: {}", local_index.index.len());
        println!("  Total free pads: {}", local_index.free_pads.len());

        Ok(())
    } else {
        // --- Regular Sync Logic ---
        let pb_read = mp.add(ProgressBar::new_spinner());
        pb_read.set_style(spinner_style.clone());
        pb_read.set_prefix("1/6");
        pb_read.set_message("Reading local index cache...");
        pb_read.enable_steady_tick(Duration::from_millis(100));
        let local_index_opt = read_local_index(network).await.map_err(|e| {
            error!("Failed to read local cache during sync: {}", e);
            pb_read.finish_with_message("Failed to read local cache.");
            CliError::from(e)
        })?;
        let local_index = local_index_opt.unwrap_or_else(|| {
            warn!("Local cache not found or empty, starting sync with default empty index.");
            MasterIndexStorage::default()
        });
        pb_read.finish_with_message("Read local index cache.");

        let pb_fetch = mp.add(ProgressBar::new_spinner());
        pb_fetch.set_style(spinner_style.clone());
        pb_fetch.set_prefix("2/6");
        pb_fetch.set_message("Fetching remote index...");
        pb_fetch.enable_steady_tick(Duration::from_millis(100));
        let remote_index = match mutant.fetch_remote_master_index().await {
            Ok(index) => {
                pb_fetch.finish_with_message("Fetched remote index.");
                index
            }
            Err(mutant_lib::error::Error::MasterIndexNotFound) => {
                pb_fetch.set_message("Remote index not found, creating...");
                warn!("Remote master index not found. Creating remote index from current state.");
                // The MutAnt instance should already hold the correct default index
                // (including size) from initialization.
                // Save the current state to create the remote index.
                if let Err(e) = mutant.save_master_index().await {
                    error!(
                        "Failed to create default remote index from current state during sync: {}",
                        e
                    );
                    pb_fetch.finish_with_message("Failed to create remote index.");
                    return Err(CliError::from(e));
                }
                pb_fetch.set_message("Created remote index, fetching again...");

                // Now that it's created, fetch it again to use for the merge.
                match mutant.fetch_remote_master_index().await {
                    Ok(newly_created_index) => {
                        pb_fetch.finish_with_message("Fetched newly created remote index.");
                        newly_created_index
                    }
                    Err(e) => {
                        error!(
                            "Failed to fetch the newly created remote index during sync: {}",
                            e
                        );
                        pb_fetch.finish_with_message("Failed to fetch created remote index.");
                        return Err(CliError::from(e));
                    }
                }
            }
            Err(e) => {
                error!("Failed to fetch remote index during sync: {}", e);
                pb_fetch.finish_with_message("Failed to fetch remote index.");
                return Err(CliError::from(e));
            }
        };
        pb_fetch.finish_with_message("Fetched remote index."); // Ensure finish in case of success path

        let pb_merge = mp.add(ProgressBar::new_spinner());
        pb_merge.set_style(spinner_style.clone());
        pb_merge.set_prefix("3/6");
        pb_merge.set_message("Merging local and remote indices...");
        pb_merge.enable_steady_tick(Duration::from_millis(100));

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

        pb_merge.finish_with_message("Merged indices.");

        let pb_update_mem = mp.add(ProgressBar::new_spinner());
        pb_update_mem.set_style(spinner_style.clone());
        pb_update_mem.set_prefix("4/6");
        pb_update_mem.set_message("Updating in-memory index...");
        pb_update_mem.enable_steady_tick(Duration::from_millis(100));
        mutant
            .update_internal_master_index(merged_index.clone())
            .await
            .map_err(|e| {
                pb_update_mem.finish_with_message("Failed to update in-memory index.");
                CliError::from(e)
            })?;
        pb_update_mem.finish_with_message("Updated in-memory index.");

        let pb_write_cache = mp.add(ProgressBar::new_spinner());
        pb_write_cache.set_style(spinner_style.clone());
        pb_write_cache.set_prefix("5/6");
        pb_write_cache.set_message("Writing merged index to local cache...");
        pb_write_cache.enable_steady_tick(Duration::from_millis(100));
        write_local_index(&merged_index, network)
            .await
            .map_err(|e| {
                error!("Failed to write merged index to local cache: {}", e);
                pb_write_cache.finish_with_message("Failed to write local cache.");
                CliError::from(e)
            })?;
        pb_write_cache.finish_with_message("Wrote merged index to local cache.");

        let pb_save_remote = mp.add(ProgressBar::new_spinner());
        pb_save_remote.set_style(spinner_style.clone());
        pb_save_remote.set_prefix("6/6");
        pb_save_remote.set_message("Saving merged index to remote storage...");
        pb_save_remote.enable_steady_tick(Duration::from_millis(100));
        mutant.save_master_index().await.map_err(|e| {
            error!("Failed to save merged index to remote storage: {}", e);
            pb_save_remote.finish_with_message("Failed to save merged index.");
            CliError::from(e)
        })?;
        pb_save_remote.finish_with_message("Saved merged index to remote.");

        mp.clear().expect("Failed to clear multi progress bar");
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
