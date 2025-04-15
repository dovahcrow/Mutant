use crate::index::{IndexManager, PadInfo};
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::{ScratchpadAddress, SecretKey};
use blsttc::SK_SIZE;
use log::{debug, trace, warn};

/// Acquires a single free pad from the index manager.
pub(crate) async fn acquire_free_pad(
    index_manager: &dyn IndexManager,
) -> Result<(ScratchpadAddress, SecretKey), PadLifecycleError> {
    trace!("Pool: Attempting to acquire free pad");
    match index_manager.take_free_pad().await? {
        Some((address, key_bytes)) => {
            // Attempt to parse the secret key bytes
            let key_bytes_len = key_bytes.len(); // Get length before moving
            let key_array: [u8; SK_SIZE] = key_bytes.try_into().map_err(|_| {
                warn!(
                    "Failed to convert key bytes to fixed-size array for pad {}. Expected size {}, got {}. Pad might be unusable.",
                    address, SK_SIZE, key_bytes_len // Use stored length
                );
                PadLifecycleError::InternalError(format!(
                    "Corrupt key data (invalid size) for free pad {}",
                    address
                ))
            })?;
            let key = SecretKey::from_bytes(key_array).map_err(|e| {
                warn!(
                    "Failed to parse SecretKey bytes for pad {}: {}. Pad might be unusable.",
                    address, e
                );
                // Return an error indicating the pad data in the index was corrupt
                PadLifecycleError::InternalError(format!(
                    "Corrupt key data (parse failed) for free pad {}",
                    address
                ))
            })?;
            debug!("Pool: Acquired free pad {}", address);
            Ok((address, key))
        }
        None => {
            debug!("Pool: No free pads available");
            Err(PadLifecycleError::PadAcquisitionFailed(
                "No free pads available in the index".to_string(),
            ))
        }
    }
}

/// Releases a list of pads (represented by PadInfo) back to the free list via the index manager.
/// This function expects the caller (e.g., DataManager) to have the secret keys if needed elsewhere,
/// as only the address is stored in PadInfo. It fetches the key bytes from the index.
/// NOTE: This assumes the keys *were* stored correctly in the index's free_pads list initially.
/// If pads are generated dynamically later, this needs adjustment.
pub(crate) async fn release_pads_to_free(
    index_manager: &dyn IndexManager,
    pads_info: Vec<PadInfo>, // Contains address and chunk index
    // We need a way to get the key bytes. Can we assume they are findable?
    // Option 1: Require keys to be passed in (complex for caller).
    // Option 2: Look up keys (not stored in KeyInfo).
    // Option 3: Assume the index manager handles finding/re-adding keys (requires IndexManager change).
    // Let's assume for now that the IndexManager's add_free_pad *needs* the key bytes.
    // This implies the caller must somehow provide them or they must be retrieved.
    // The original code likely put the (addr, key_bytes) tuple back.
    // This requires the caller (DataManager::remove/update) to retrieve the key bytes.
    // Let's modify this function signature to expect the key bytes.
    pad_keys: &std::collections::HashMap<ScratchpadAddress, Vec<u8>>, // Map address to key bytes
) -> Result<(), PadLifecycleError> {
    trace!("Pool: Releasing {} pads to free list", pads_info.len());
    for pad in pads_info {
        match pad_keys.get(&pad.address) {
            Some(key_bytes) => {
                debug!("Pool: Releasing pad {}", pad.address);
                index_manager
                    .add_free_pad(pad.address, key_bytes.clone())
                    .await?;
            }
            None => {
                // This indicates a logic error somewhere - we are trying to release a pad
                // but don't have its key bytes provided.
                warn!(
                    "Pool: Cannot release pad {}, key bytes not provided.",
                    pad.address
                );
                // Should this be an error or just a warning? Error seems appropriate.
                return Err(PadLifecycleError::InternalError(format!(
                    "Key bytes not found for pad {} during release",
                    pad.address
                )));
            }
        }
    }
    Ok(())
}

// Note: Pad generation logic is omitted as per plan refinement.
// If needed later, a function like `generate_new_pads` would be added here,
// potentially interacting with autonomi key generation and adding them
// to the index manager's pending or free list.
