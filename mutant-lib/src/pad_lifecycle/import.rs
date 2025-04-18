use crate::index::manager::DefaultIndexManager;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::{ScratchpadAddress, SecretKey};
use hex;
use log::{debug, info};

/// Imports a pad into the index manager's free list using its private key.
///
/// This derives the pad address from the private key and adds it to the free list.
/// It assumes the pad exists on the network and is usable.
///
/// # Arguments
///
/// * `index_manager` - A reference to the `DefaultIndexManager`.
/// * `private_key_hex` - The private key of the pad to import, encoded as a hexadecimal string.
///
/// # Errors
///
/// Returns `PadLifecycleError::InvalidInput` if the hex string is invalid or the key
/// has the wrong length.
/// Returns `PadLifecycleError::Index` if adding the pad to the index manager fails.
pub(crate) async fn import_pad(
    index_manager: &DefaultIndexManager,
    private_key_hex: &str,
) -> Result<(), PadLifecycleError> {
    info!(
        "Import: Attempting import for key starting with: {}...",
        private_key_hex.get(..8).unwrap_or("invalid")
    );

    let key_bytes = hex::decode(private_key_hex).map_err(|e| {
        PadLifecycleError::InvalidInput(format!("Invalid private key hex for import: {}", e))
    })?;

    let key_array: [u8; 32] = key_bytes.as_slice().try_into().map_err(|_| {
        PadLifecycleError::InvalidInput(format!(
            "Private key for import has incorrect length. Expected 32 bytes, got {}",
            key_bytes.len()
        ))
    })?;
    let secret_key = SecretKey::from_bytes(key_array).map_err(|e| {
        PadLifecycleError::InvalidInput(format!("Invalid secret key bytes for import: {}", e))
    })?;

    let public_key = secret_key.public_key();
    let pad_address = ScratchpadAddress::new(public_key);
    debug!("Import: Derived address {} for import.", pad_address);

    index_manager.add_free_pad(pad_address, key_bytes).await?;

    info!(
        "Import: Successfully added pad {} to free list via IndexManager.",
        pad_address
    );
    Ok(())
}
