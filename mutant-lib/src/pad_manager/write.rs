use super::PadManager;
use crate::cache::write_local_index;
use crate::error::Error;
use crate::events::PutCallback;
use crate::mutant::data_structures::{KeyStorageInfo, MasterIndexStorage};
use crate::storage::{network, ContentType, Storage};
use autonomi::{ScratchpadAddress, SecretKey};
use chrono;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

// mod concurrent; // Removed module
// mod reservation; // Removed module

// Removed unused type alias
// pub(crate) type PadInfoAlias = (ScratchpadAddress, Vec<u8>);

/// Writes a single data chunk to a specified scratchpad.
///
/// This function acts as a wrapper around the storage layer's create/update operations,
/// handling the choice between creating a new pad or updating an existing one.
/// It relies on the underlying storage functions to handle confirmation loops.
///
/// # Arguments
/// * `storage` - Reference to the storage interface.
/// * `address` - The address of the scratchpad to write to.
/// * `key_bytes` - The encryption key bytes for the scratchpad.
/// * `data_chunk` - The chunk of data to write.
/// * `is_new_pad` - Boolean indicating whether to create a new pad (`true`) or update an existing one (`false`).
/// * `key_str` - For logging/event context.
/// * `pad_index` - For logging/event context.
/// * `callback_arc` - For callback infrastructure.
/// * `total_pads_committed_arc` - For callback infrastructure.
/// * `total_pads_expected` - For callback infrastructure.
/// * `scratchpad_size` - The size of the scratchpad.
///
/// # Returns
/// Returns `Ok(())` on successful write and confirmation, otherwise a `crate::error::Error`.
#[allow(clippy::too_many_arguments)]
pub async fn write_chunk(
    storage: &Storage,
    address: ScratchpadAddress,
    key_bytes: &[u8],
    data_chunk: &[u8],
    is_new_pad: bool,
    key_str: &str,
    pad_index: usize,
    callback_arc: &Arc<Mutex<Option<PutCallback>>>,
    total_pads_committed_arc: &Arc<Mutex<u64>>,
    total_pads_expected: usize,
    scratchpad_size: usize, // Use this parameter again
) -> Result<(), Error> {
    debug!(
        "write_chunk[{}][Pad {}]: Address={}, IsNew={}",
        key_str,
        pad_index,
        address,
        // key_bytes.len(), // Removed for brevity
        // data_chunk.len(), // Removed for brevity
        is_new_pad
    );

    let key_array: [u8; 32] = key_bytes.try_into().map_err(|_| {
        Error::InvalidInput(format!(
            "Invalid secret key byte length: expected 32, got {}",
            key_bytes.len()
        ))
    })?;
    let secret_key = SecretKey::from_bytes(key_array)
        .map_err(|e| Error::InvalidInput(format!("Invalid secret key bytes: {}", e)))?;

    let client = storage.get_client().await?;

    if is_new_pad {
        // Call create, which now handles its own verification and commit event
        let wallet = storage.wallet();
        let payment_option = autonomi::client::payment::PaymentOption::from(wallet);
        network::create_scratchpad_static(
            client,
            &secret_key,
            data_chunk, // Pass initial data directly
            ContentType::DataChunk as u64,
            payment_option,
            key_str,
            pad_index,
            callback_arc,
            total_pads_committed_arc,
            total_pads_expected,
        )
        .await?;
        // create returns address, we discard it and return Ok(()) on success
        Ok(())
    } else {
        // Call update with progress, which handles verification and commit event
        let bytes_in_chunk = data_chunk.len() as u64;
        // Estimate total size using passed scratchpad_size
        let total_size_overall = scratchpad_size as u64 * total_pads_expected as u64;
        network::update_scratchpad_internal_static_with_progress(
            client,
            &secret_key,
            data_chunk,
            ContentType::DataChunk as u64,
            key_str,
            pad_index,
            callback_arc,
            &Arc::new(Mutex::new(0)), // Dummy total_bytes_uploaded
            bytes_in_chunk,
            total_size_overall,
            false, // is_new_pad is false for updates
            total_pads_committed_arc,
            total_pads_expected,
        )
        .await
    }
}

// Removed unused helper function
// fn calculate_needed_pads(...) { ... }

// Removed unused PadManager impl block
// impl PadManager {
//     fn acquire_resources_for_write(...) { ... }
//     pub async fn allocate_and_write(...) { ... }
// }
