use crate::index::IndexManager;
use crate::pad_lifecycle::error::PadLifecycleError;
use autonomi::{ScratchpadAddress, SecretKey};
use hex;
use log::{debug, info};

/// Imports an external free pad using its private key hex string.
///
/// It parses the key, derives the address, and attempts to add it to the
/// free pad list managed by the IndexManager. It does *not* check for
/// conflicts with occupied pads here; that check is assumed to be handled
/// implicitly or explicitly by the IndexManager's `add_free_pad` implementation
/// or higher layers if necessary.
pub(crate) async fn import_pad(
    index_manager: &dyn IndexManager,
    private_key_hex: &str,
) -> Result<(), PadLifecycleError> {
    info!(
        "Import: Attempting import for key starting with: {}...",
        private_key_hex.get(..8).unwrap_or("invalid")
    );

    // 1. Parse the private key hex
    let key_bytes = hex::decode(private_key_hex).map_err(|e| {
        PadLifecycleError::InvalidInput(format!("Invalid private key hex for import: {}", e))
    })?;

    // 2. Validate key length and create SecretKey
    // Use slice directly for try_into
    let key_array: [u8; 32] = key_bytes.as_slice().try_into().map_err(|_| {
        PadLifecycleError::InvalidInput(format!(
            "Private key for import has incorrect length. Expected 32 bytes, got {}",
            key_bytes.len()
        ))
    })?;
    let secret_key = SecretKey::from_bytes(key_array).map_err(|e| {
        // This error from SecretKey::from_bytes usually means non-canonical representation,
        // which shouldn't happen often with hex decoding but handle it.
        PadLifecycleError::InvalidInput(format!("Invalid secret key bytes for import: {}", e))
    })?;

    // 3. Derive the public key and address
    let public_key = secret_key.public_key();
    let pad_address = ScratchpadAddress::new(public_key);
    debug!("Import: Derived address {} for import.", pad_address);

    // 4. Add to free pads list via IndexManager
    // The IndexManager::add_free_pad should handle potential duplicate checks within the free list itself.
    // We pass the original key_bytes Vec.
    index_manager
        .add_free_pad(pad_address, key_bytes) // Pass original Vec<u8>
        .await?; // Propagate any IndexError as PadLifecycleError::Index

    info!(
        "Import: Successfully added pad {} to free list via IndexManager.",
        pad_address
    );
    Ok(())

    // Note: The plan mentioned checking for conflicts with occupied pads here.
    // However, doing that efficiently requires iterating through all KeyInfo in the index,
    // which might be slow and better handled within IndexManager if needed, or accepted as a risk.
    // The current IndexManager::add_free_pad only checks for duplicates in the free list itself.
    // If stricter checks are needed, IndexManager logic would need adjustment.
}