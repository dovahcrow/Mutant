use crate::index::error::IndexError;
use crate::index::structure::{KeyInfo, MasterIndex, PadInfo, DEFAULT_SCRATCHPAD_SIZE};
use crate::types::{KeyDetails, StorageStats};
use autonomi::ScratchpadAddress;
use log::{debug, trace, warn};
use std::collections::HashMap;

// --- Internal Query & Modification Functions ---
// These functions operate directly on the MasterIndex state and are
// intended to be called while holding a lock (e.g., MutexGuard).

/// Retrieves information for a specific key.
pub(crate) fn get_key_info_internal<'a>(index: &'a MasterIndex, key: &str) -> Option<&'a KeyInfo> {
    trace!("Query: get_key_info_internal for key '{}'", key);
    index.index.get(key)
}

/// Inserts or updates information for a specific key.
pub(crate) fn insert_key_info_internal(
    index: &mut MasterIndex,
    key: String,
    info: KeyInfo,
) -> Result<(), IndexError> {
    trace!("Query: insert_key_info_internal for key '{}'", key);
    // TODO: Add validation? E.g., ensure pad list isn't empty if size > 0?
    index.index.insert(key, info);
    Ok(())
}

/// Removes information for a specific key, returning the old info if it existed.
pub(crate) fn remove_key_info_internal(index: &mut MasterIndex, key: &str) -> Option<KeyInfo> {
    trace!("Query: remove_key_info_internal for key '{}'", key);
    index.index.remove(key)
}

/// Lists all user keys currently stored in the index.
pub(crate) fn list_keys_internal(index: &MasterIndex) -> Vec<String> {
    trace!("Query: list_keys_internal");
    index.index.keys().cloned().collect()
    // Consider filtering out internal keys if any are added later
}

/// Retrieves detailed information for a specific key.
pub(crate) fn get_key_details_internal(index: &MasterIndex, key: &str) -> Option<KeyDetails> {
    trace!("Query: get_key_details_internal for key '{}'", key);
    index.index.get(key).map(|info| {
        let percentage = if !info.is_complete && !info.pads.is_empty() {
            Some((info.populated_pads_count as f32 / info.pads.len() as f32) * 100.0)
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

/// Retrieves detailed information for all keys.
pub(crate) fn list_all_key_details_internal(index: &MasterIndex) -> Vec<KeyDetails> {
    trace!("Query: list_all_key_details_internal");
    index
        .index
        .iter()
        .map(|(key, info)| {
            let percentage = if !info.is_complete && !info.pads.is_empty() {
                Some((info.populated_pads_count as f32 / info.pads.len() as f32) * 100.0)
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

/// Calculates storage statistics based on the current index state.
pub(crate) fn get_stats_internal(index: &MasterIndex) -> Result<StorageStats, IndexError> {
    trace!("Query: get_stats_internal");
    let scratchpad_size = index.scratchpad_size;
    if scratchpad_size == 0 {
        warn!("Cannot calculate stats: Scratchpad size in index is zero.");
        // Return an error or default stats? Error seems safer.
        return Err(IndexError::InconsistentState(
            "Scratchpad size in index is zero".to_string(),
        ));
    }

    let free_pads_count = index.free_pads.len();
    let pending_verification_pads_count = index.pending_verification_pads.len();
    let mut occupied_pads_count = 0;
    let mut occupied_data_size_total: u64 = 0;

    for key_info in index.index.values() {
        occupied_pads_count += key_info.pads.len();
        occupied_data_size_total += key_info.data_size as u64;
    }

    let total_pads_count = occupied_pads_count + free_pads_count + pending_verification_pads_count;

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
    })
}

/// Adds a pad to the free list. Checks for duplicates.
pub(crate) fn add_free_pad_internal(
    index: &mut MasterIndex,
    address: ScratchpadAddress,
    key_bytes: Vec<u8>,
) -> Result<(), IndexError> {
    trace!("Query: add_free_pad_internal for address '{}'", address);
    if index.free_pads.iter().any(|(addr, _)| *addr == address) {
        warn!("Attempted to add duplicate pad to free list: {}", address);
        // Decide on behavior: error or ignore? Ignore seems reasonable for robustness.
        return Ok(());
        // return Err(IndexError::InconsistentState(format!("Pad {} already in free list", address)));
    }
    // Also check occupied pads? This might be expensive. Assume checks happen higher up (e.g., during import).
    index.free_pads.push((address, key_bytes));
    Ok(())
}

/// Takes a single pad from the free list, if available.
pub(crate) fn take_free_pad_internal(
    index: &mut MasterIndex,
) -> Option<(ScratchpadAddress, Vec<u8>)> {
    trace!("Query: take_free_pad_internal");
    index.free_pads.pop()
}

/// Adds multiple pads to the pending verification list.
pub(crate) fn add_pending_pads_internal(
    index: &mut MasterIndex,
    pads: Vec<(ScratchpadAddress, Vec<u8>)>,
) -> Result<(), IndexError> {
    trace!("Query: add_pending_pads_internal ({} pads)", pads.len());
    // TODO: Check for duplicates within the input list and existing pending list?
    index.pending_verification_pads.extend(pads);
    Ok(())
}

/// Takes all pads from the pending verification list.
pub(crate) fn take_pending_pads_internal(
    index: &mut MasterIndex,
) -> Vec<(ScratchpadAddress, Vec<u8>)> {
    trace!("Query: take_pending_pads_internal");
    std::mem::take(&mut index.pending_verification_pads)
}

/// Updates free/pending lists based on verification results.
/// Moves verified pads to the free list.
pub(crate) fn update_pad_lists_internal(
    index: &mut MasterIndex,
    verified: Vec<(ScratchpadAddress, Vec<u8>)>,
    _failed_count: usize, // Parameter kept for potential future use (e.g., logging)
) -> Result<(), IndexError> {
    trace!(
        "Query: update_pad_lists_internal ({} verified)",
        verified.len()
    );
    // Add verified pads to the free list, checking for duplicates just in case
    for (addr, key) in verified {
        if !index.free_pads.iter().any(|(a, _)| *a == addr) {
            index.free_pads.push((addr, key));
        } else {
            warn!(
                "Pad {} verified but already found in free list during update.",
                addr
            );
        }
    }
    // Pending list should already be empty (taken by take_pending_pads_internal)
    Ok(())
}

/// Resets the index to a default state.
pub(crate) fn reset_index_internal(index: &mut MasterIndex) {
    trace!("Query: reset_index_internal");
    *index = MasterIndex {
        scratchpad_size: index.scratchpad_size.max(DEFAULT_SCRATCHPAD_SIZE), // Keep existing or default size
        ..Default::default()
    };
    debug!("Index reset to default state.");
}
