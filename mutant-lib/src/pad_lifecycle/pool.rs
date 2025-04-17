use crate::index::IndexManager;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::{ScratchpadAddress, SecretKey};
use blsttc::SK_SIZE;
use log::{debug, trace, warn};

pub(crate) async fn acquire_free_pad(
    index_manager: &dyn IndexManager,
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
