use crate::index::error::IndexError;
use crate::index::structure::{KeyInfo, MasterIndex, PadStatus, DEFAULT_SCRATCHPAD_SIZE};
use crate::types::{KeyDetails, StorageStats};
use autonomi::ScratchpadAddress;
use log::{debug, trace, warn};

pub(crate) fn get_key_info_internal<'a>(index: &'a MasterIndex, key: &str) -> Option<&'a KeyInfo> {
    trace!("Query: get_key_info_internal for key '{}'", key);
    index.index.get(key)
}

pub(crate) fn insert_key_info_internal(
    index: &mut MasterIndex,
    key: String,
    info: KeyInfo,
) -> Result<(), IndexError> {
    trace!("Query: insert_key_info_internal for key '{}'", key);

    index.index.insert(key, info);
    Ok(())
}

pub(crate) fn remove_key_info_internal(index: &mut MasterIndex, key: &str) -> Option<KeyInfo> {
    trace!("Query: remove_key_info_internal for key '{}'", key);
    index.index.remove(key)
}

pub(crate) fn list_keys_internal(index: &MasterIndex) -> Vec<String> {
    trace!("Query: list_keys_internal");
    index.index.keys().cloned().collect()
}

pub(crate) fn get_key_details_internal(index: &MasterIndex, key: &str) -> Option<KeyDetails> {
    trace!("Query: get_key_details_internal for key '{}'", key);
    index.index.get(key).map(|info| {
        let percentage = if !info.is_complete && !info.pads.is_empty() {
            let confirmed_count = info
                .pads
                .iter()
                .filter(|p| p.status == PadStatus::Confirmed)
                .count();
            Some((confirmed_count as f32 / info.pads.len() as f32) * 100.0)
        } else {
            None
        };
        KeyDetails {
            key: key.to_string(),
            size: info.data_size,
            modified: info.modified,
            is_finished: info.is_complete,
            completion_percentage: percentage,
        }
    })
}

pub(crate) fn list_all_key_details_internal(index: &MasterIndex) -> Vec<KeyDetails> {
    trace!("Query: list_all_key_details_internal");
    index
        .index
        .iter()
        .map(|(key, info)| {
            let percentage = if !info.is_complete && !info.pads.is_empty() {
                let confirmed_count = info
                    .pads
                    .iter()
                    .filter(|p| p.status == PadStatus::Confirmed)
                    .count();
                Some((confirmed_count as f32 / info.pads.len() as f32) * 100.0)
            } else {
                None
            };
            KeyDetails {
                key: key.clone(),
                size: info.data_size,
                modified: info.modified,
                is_finished: info.is_complete,
                completion_percentage: percentage,
            }
        })
        .collect()
}

pub(crate) fn get_stats_internal(index: &MasterIndex) -> Result<StorageStats, IndexError> {
    trace!("Query: get_stats_internal");
    let scratchpad_size = index.scratchpad_size;
    if scratchpad_size == 0 {
        warn!("Cannot calculate stats: Scratchpad size in index is zero.");
        return Err(IndexError::InconsistentState(
            "Scratchpad size in index is zero".to_string(),
        ));
    }

    let free_pads_count = index.free_pads.len();
    let pending_verification_pads_count = index.pending_verification_pads.len();

    let mut occupied_pads_count = 0;
    let mut occupied_data_size_total: u64 = 0;
    let mut allocated_written_pads_count = 0;

    let mut incomplete_keys_count = 0;
    let mut incomplete_keys_data_bytes = 0;
    let mut incomplete_keys_total_pads = 0;
    let mut incomplete_keys_pads_generated = 0;
    let mut _incomplete_keys_pads_allocated = 0;
    let mut incomplete_keys_pads_written = 0;
    let mut incomplete_keys_pads_confirmed = 0;

    for key_info in index.index.values() {
        if key_info.is_complete {
            occupied_pads_count += key_info.pads.len();
            occupied_data_size_total += key_info.data_size as u64;
        } else {
            incomplete_keys_count += 1;
            incomplete_keys_data_bytes += key_info.data_size as u64;
            incomplete_keys_total_pads += key_info.pads.len();

            for pad_info in &key_info.pads {
                match pad_info.status {
                    PadStatus::Generated => incomplete_keys_pads_generated += 1,
                    PadStatus::Allocated => {
                        _incomplete_keys_pads_allocated += 1;
                        allocated_written_pads_count += 1;
                    }
                    PadStatus::Written => {
                        incomplete_keys_pads_written += 1;
                        allocated_written_pads_count += 1;
                    }
                    PadStatus::Confirmed => {
                        incomplete_keys_pads_confirmed += 1;

                        occupied_pads_count += 1;
                    }
                }
            }

            occupied_data_size_total += key_info.data_size as u64;
        }
    }

    let total_pads_count = occupied_pads_count
        + allocated_written_pads_count
        + free_pads_count
        + pending_verification_pads_count;

    let scratchpad_size_u64 = scratchpad_size as u64;

    let occupied_pad_space_bytes = occupied_pads_count as u64 * scratchpad_size_u64;
    let free_pad_space_bytes = free_pads_count as u64 * scratchpad_size_u64;
    let total_space_bytes = total_pads_count as u64 * scratchpad_size_u64;

    let wasted_space_bytes = occupied_pad_space_bytes.saturating_sub(occupied_data_size_total);

    Ok(StorageStats {
        scratchpad_size,
        total_pads: total_pads_count,
        occupied_pads: occupied_pads_count,
        free_pads: free_pads_count,
        pending_verification_pads: pending_verification_pads_count,
        total_space_bytes,
        occupied_pad_space_bytes,
        free_pad_space_bytes,
        occupied_data_bytes: occupied_data_size_total,
        wasted_space_bytes,
        incomplete_keys_count,
        incomplete_keys_data_bytes,
        incomplete_keys_total_pads,
        incomplete_keys_pads_generated,
        incomplete_keys_pads_written,
        incomplete_keys_pads_confirmed,
    })
}

pub(crate) fn add_free_pad_with_counter_internal(
    index: &mut MasterIndex,
    address: ScratchpadAddress,
    key_bytes: Vec<u8>,
    counter: u64,
) -> Result<(), IndexError> {
    trace!(
        "Query: add_free_pad_with_counter_internal for address '{}' (counter: {})",
        address,
        counter
    );
    if index.free_pads.iter().any(|(addr, _, _)| *addr == address) {
        warn!("Attempted to add duplicate pad to free list: {}", address);
        return Ok(());
    }
    index.free_pads.push((address, key_bytes, counter));
    Ok(())
}

pub(crate) fn take_free_pad_internal(
    index: &mut MasterIndex,
) -> Option<(ScratchpadAddress, Vec<u8>, u64)> {
    trace!("Query: take_free_pad_internal");
    index.free_pads.pop()
}

pub(crate) fn add_free_pads_with_counters_internal(
    index: &mut MasterIndex,
    pads: Vec<(ScratchpadAddress, Vec<u8>, u64)>,
) -> Result<(), IndexError> {
    trace!(
        "Query: add_free_pads_with_counters_internal ({} pads)",
        pads.len()
    );
    for (address, key_bytes, counter) in pads {
        if !index.free_pads.iter().any(|(addr, _, _)| *addr == address) {
            index.free_pads.push((address, key_bytes, counter));
        } else {
            warn!(
                "Attempted to add duplicate pad to free list via batch: {}",
                address
            );
        }
    }
    Ok(())
}

pub(crate) fn add_pending_verification_pads_internal(
    index: &mut MasterIndex,
    pads: Vec<(ScratchpadAddress, Vec<u8>)>,
) -> Result<(), IndexError> {
    trace!(
        "Query: add_pending_verification_pads_internal ({} pads)",
        pads.len()
    );
    for (address, key_bytes) in pads {
        if !index
            .pending_verification_pads
            .iter()
            .any(|(addr, _)| *addr == address)
        {
            index.pending_verification_pads.push((address, key_bytes));
        } else {
            warn!(
                "Attempted to add duplicate pad to pending list via batch: {}",
                address
            );
        }
    }
    Ok(())
}

pub(crate) fn take_pending_pads_internal(
    index: &mut MasterIndex,
) -> Vec<(ScratchpadAddress, Vec<u8>)> {
    trace!("Query: take_pending_pads_internal");
    std::mem::take(&mut index.pending_verification_pads)
}

pub(crate) fn remove_from_pending_internal(
    index: &mut MasterIndex,
    address_to_remove: &ScratchpadAddress,
) -> Result<(), IndexError> {
    trace!(
        "Query: remove_from_pending_internal for address '{}'",
        address_to_remove
    );
    index
        .pending_verification_pads
        .retain(|(addr, _)| addr != address_to_remove);
    Ok(())
}

pub(crate) fn update_pad_status_internal(
    index: &mut MasterIndex,
    key: &str,
    pad_address: &ScratchpadAddress,
    new_status: PadStatus,
) -> Result<(), IndexError> {
    trace!(
        "Query: update_pad_status_internal for key '{}', pad '{}', status {:?}",
        key,
        pad_address,
        new_status
    );
    if let Some(key_info) = index.index.get_mut(key) {
        if let Some(pad_info) = key_info.pads.iter_mut().find(|p| p.address == *pad_address) {
            pad_info.status = new_status;
            Ok(())
        } else {
            warn!(
                "Attempted to update status for pad {} which is not found in key '{}'",
                pad_address, key
            );
            Err(IndexError::InconsistentState(format!(
                "Pad {} not found in key {}",
                pad_address, key
            )))
        }
    } else {
        Err(IndexError::KeyNotFound(key.to_string()))
    }
}

pub(crate) fn mark_key_complete_internal(
    index: &mut MasterIndex,
    key: &str,
) -> Result<(), IndexError> {
    trace!("Query: mark_key_complete_internal for key '{}'", key);
    if let Some(key_info) = index.index.get_mut(key) {
        key_info.is_complete = true;
        Ok(())
    } else {
        Err(IndexError::KeyNotFound(key.to_string()))
    }
}

pub(crate) fn reset_index_internal(index: &mut MasterIndex) {
    trace!("Query: reset_index_internal");
    *index = MasterIndex {
        scratchpad_size: index.scratchpad_size.max(DEFAULT_SCRATCHPAD_SIZE),
        ..Default::default()
    };
    debug!("Index reset to default state.");
}

pub(crate) fn add_pending_pads_internal(
    index: &mut MasterIndex,
    pads: Vec<(ScratchpadAddress, Vec<u8>)>,
) -> Result<(), IndexError> {
    trace!(
        "Query: add_pending_pads_internal adding {} pads",
        pads.len()
    );

    index.pending_verification_pads.extend(pads);
    Ok(())
}
