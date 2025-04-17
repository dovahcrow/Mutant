use crate::app::CliError;
use crate::callbacks::progress::StyledProgressBar;
use indicatif::{MultiProgress, ProgressDrawTarget};
use log::{debug, error, info, trace, warn};
use mutant_lib::{Error as LibError, IndexError, MutAnt, ScratchpadAddress};
use std::collections::{HashMap, HashSet};

pub async fn handle_sync(mutant: MutAnt, push_force: bool) -> Result<(), CliError> {
    info!("Starting synchronization process...");
    let _network = mutant.get_network_choice();

    let mp = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());
    let pb = StyledProgressBar::new_for_steps(&mp);

    if push_force {
        let total_steps = 1;
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting push-force sync...".to_string());

        pb.set_position(1);
        pb.set_message("Saving current index to remote (force)...".to_string());
        mutant.save_master_index().await.map_err(|e| {
            let msg = format!("Failed to save index to remote: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;

        pb.finish_with_message(
            "Push-force sync complete. Remote index overwritten with current in-memory state.",
        );

        Ok(())
    } else {
        let total_steps = 5;
        pb.set_length(total_steps);
        pb.set_position(0);
        pb.set_message("Starting regular sync...".to_string());

        pb.set_position(1);
        pb.set_message("Getting current in-memory index...".to_string());
        let local_index = mutant.get_index_copy().await.map_err(|e| {
            let msg = format!("Failed to get current index state: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;

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
                if let Err(e) = mutant.save_master_index().await {
                    let msg = format!("Failed to create remote index: {}", e);
                    error!("{}", msg);
                    pb.abandon_with_message(msg.clone());
                    return Err(CliError::from(e));
                }
                info!("Successfully created remote index from in-memory state.");
                local_index.clone()
            }
            Err(e) => {
                let msg = format!("Failed to fetch remote index: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                return Err(CliError::from(e));
            }
        };

        pb.set_position(3);
        pb.set_message("Merging in-memory and remote indices...".to_string());

        let mut merged_index = remote_index.clone();
        let mut local_keys_added = 0;
        let mut remote_keys_found = 0;

        for (key, local_info) in local_index.index.iter() {
            if !merged_index.index.contains_key(key) {
                debug!("Sync: Adding key '{}' from local to merged index.", key);
                merged_index.index.insert(key.clone(), local_info.clone());
                local_keys_added += 1;
            } else {
                remote_keys_found += 1;
                trace!("Sync: Key '{}' exists in both. Using remote version.", key);
            }
        }

        let occupied_pads: HashSet<ScratchpadAddress> = merged_index
            .index
            .values()
            .flat_map(|key_info| key_info.pads.iter().map(|pad_info| pad_info.address))
            .collect();

        let mut potential_free_pads_map: HashMap<ScratchpadAddress, (Vec<u8>, u64)> =
            HashMap::new();
        potential_free_pads_map.extend(
            local_index
                .free_pads
                .iter()
                .map(|(addr, key, counter)| (*addr, (key.clone(), *counter))),
        );
        potential_free_pads_map.extend(
            remote_index
                .free_pads
                .iter()
                .map(|(addr, key, counter)| (*addr, (key.clone(), *counter))),
        );

        let final_free_pads: Vec<(ScratchpadAddress, Vec<u8>, u64)> = potential_free_pads_map
            .into_iter()
            .filter(|(addr, _)| !occupied_pads.contains(addr))
            .map(|(addr, (key, counter))| (addr, key, counter))
            .collect();

        let remote_pads_addr_set: HashSet<_> = remote_index
            .free_pads
            .iter()
            .map(|(addr, _, _)| *addr)
            .collect();
        let local_pads_added = final_free_pads
            .iter()
            .filter(|(addr, _, _)| !remote_pads_addr_set.contains(addr))
            .count();

        merged_index.free_pads = final_free_pads;

        if local_index.scratchpad_size != 0
            && local_index.scratchpad_size != remote_index.scratchpad_size
        {
            warn!(
                "Local scratchpad size ({}) differs from remote ({}). Using remote size.",
                local_index.scratchpad_size, remote_index.scratchpad_size
            );
        }
        merged_index.scratchpad_size = remote_index.scratchpad_size;

        info!(
            "Merged index: {} total keys ({} from local added), {} final free pads ({} added vs remote).",
            merged_index.index.len(),
            local_keys_added,
            merged_index.free_pads.len(),
            local_pads_added
        );

        pb.set_position(4);
        pb.set_message("Updating state and saving remote index...".to_string());
        mutant
            .update_internal_master_index(merged_index.clone())
            .await
            .map_err(|e| {
                let msg = format!("Failed to update in-memory index: {}", e);
                error!("{}", msg);
                pb.abandon_with_message(msg.clone());
                CliError::from(e)
            })?;
        mutant.save_master_index().await.map_err(|e| {
            let msg = format!("Failed to save merged index to remote: {}", e);
            error!("{}", msg);
            pb.abandon_with_message(msg.clone());
            CliError::from(e)
        })?;

        pb.set_position(5);
        pb.set_message("Updating local cache...".to_string());
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
