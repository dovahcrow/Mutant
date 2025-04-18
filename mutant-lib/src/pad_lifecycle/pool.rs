use crate::index::manager::DefaultIndexManager;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::{ScratchpadAddress, SecretKey};
use blsttc::SK_SIZE;
use log::{debug, trace, warn};

/// Acquires a single pad from the `IndexManager`'s free list.
///
/// If a pad is available, it takes it from the list, converts the stored key bytes
/// back into a `SecretKey`, and returns the address, key, and initial counter.
///
/// # Arguments
///
/// * `index_manager` - A reference to the `DefaultIndexManager`.
///
/// # Errors
///
/// Returns `PadLifecycleError::PadAcquisitionFailed` if the free list is empty.
/// Returns `PadLifecycleError::InternalError` if the key bytes stored in the index
/// are corrupt (invalid size or unable to parse as a `SecretKey`).
/// Returns `PadLifecycleError::Index` if taking the pad from the index manager fails.
///
/// # Returns
///
/// A tuple containing the `ScratchpadAddress`, `SecretKey`, and initial counter (`u64`)
/// of the acquired free pad.
pub(crate) async fn acquire_free_pad(
    index_manager: &DefaultIndexManager,
) -> Result<(ScratchpadAddress, SecretKey, u64), PadLifecycleError> {
    trace!("Pool: Attempting to acquire free pad");
    match index_manager.take_free_pad().await? {
        Some((address, key_bytes, initial_counter)) => {
            let key_bytes_len = key_bytes.len();
            let key_array: [u8; SK_SIZE] = key_bytes.try_into().map_err(|_| {
                warn!(
                    "Failed to convert key bytes to fixed-size array for pad {}. Expected size {}, got {}. Pad might be unusable.",
                    address, SK_SIZE, key_bytes_len 
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

                PadLifecycleError::InternalError(format!(
                    "Corrupt key data (parse failed) for free pad {}",
                    address
                ))
            })?;
            debug!(
                "Pool: Acquired free pad {} with initial counter {}",
                address, initial_counter
            );
            Ok((address, key, initial_counter))
        }
        None => {
            debug!("Pool: No free pads available");
            Err(PadLifecycleError::PadAcquisitionFailed(
                "No free pads available in the index".to_string(),
            ))
        }
    }
}
