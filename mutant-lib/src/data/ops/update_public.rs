use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;

// Import the helper function from the same module
use super::store_public::upload_public_data_and_create_index;

pub(crate) async fn update_public_op(
    manager: &DefaultDataManager,
    name: &str,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!(
        "DataOps: Starting update_public_op for name '{}' (remove then store)",
        name
    );
    let callback_arc = Arc::new(Mutex::new(callback));
    let data_size = data_bytes.len();
    let total_size = data_size as u64;

    // 1. Check existence and type
    debug!("Checking existence and type for '{}'...", name);
    let maybe_public_info = manager
        .index_manager
        .get_public_upload_info(name)
        .await
        .map_err(DataError::Index)?;

    if let Some(public_info) = maybe_public_info {
        // --- It exists and is public: Perform UPDATE ---
        debug!("Found existing public upload. Proceeding with update...");
        let public_index_address = public_info.address;
        let index_sk = SecretKey::from_bytes(
            public_info
                .index_secret_key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| {
                    DataError::InternalError("Invalid index secret key length".to_string())
                })?,
        )
        .map_err(|_| DataError::InternalError("Invalid index secret key format".to_string()))?;

        // 2. Chunk Data
        let chunk_size = manager.index_manager.get_scratchpad_size().await?;
        if chunk_size == 0 {
            error!("Configured scratchpad size is 0. Cannot proceed with chunking.");
            return Err(DataError::ChunkingError(
                "Invalid scratchpad size (0) configured".to_string(),
            ));
        }
        let chunks = chunk_data(data_bytes, chunk_size)?;
        let total_chunks = chunks.len();
        let _chunk_count = total_chunks;
        debug!(
            "Public data for '{}' chunked into {} pieces using size {}.",
            name, total_chunks, chunk_size
        );

        // 3. Emit Starting event
        if !invoke_put_callback(
            &mut *callback_arc.lock().await,
            PutEvent::Starting {
                total_chunks,
                initial_written_count: 0,
                initial_confirmed_count: 0,
            },
        )
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
        {
            warn!("Public update/store for '{}' cancelled at start.", name);
            return Err(DataError::OperationCancelled);
        }

        let chunk_addresses: Vec<ScratchpadAddress>;

        if total_chunks == 0 {
            debug!(
                "Public update/store '{}': No chunks to upload (empty data).",
                name
            );
            chunk_addresses = Vec::new();
        } else {
            // 4. Upload Chunks in Parallel
            let mut upload_futures = FuturesUnordered::new();
            let mut chunk_addresses_results: Vec<Option<ScratchpadAddress>> =
                vec![None; total_chunks];

            for (i, chunk) in chunks.into_iter().enumerate() {
                let network_adapter_clone = Arc::clone(&manager.network_adapter);
                let chunk_sk = SecretKey::random();
                let chunk_bytes = Bytes::from(chunk);

                upload_futures.push(async move {
                    trace!("Starting upload task for public chunk {}", i);
                    let chunk_scratchpad =
                        create_public_scratchpad(&chunk_sk, PUBLIC_DATA_ENCODING, &chunk_bytes, 0);
                    let chunk_addr = *chunk_scratchpad.address();
                    let payment = PaymentOption::Wallet((*network_adapter_clone.wallet()).clone());
                    match network_adapter_clone
                        .scratchpad_put(chunk_scratchpad, payment)
                        .await
                    {
                        Ok((_cost, addr)) => {
                            trace!("Upload task successful for public chunk {} -> {}", i, addr);
                            Ok((i, addr))
                        }
                        Err(e) => {
                            error!(
                                "Upload task failed for public chunk {} (addr: {}): {}",
                                i, chunk_addr, e
                            );
                            Err(DataError::Network(e))
                        }
                    }
                });
            }

            let mut first_error: Option<DataError> = None;
            let mut completed_count = 0;

            while let Some(result) = upload_futures.next().await {
                match result {
                    Ok((index, addr)) => {
                        debug!(
                            "Successfully uploaded public chunk {} ({}) for {}",
                            index, addr, name
                        );
                        chunk_addresses_results[index] = Some(addr);
                        completed_count += 1;

                        if !invoke_put_callback(
                            &mut *callback_arc.lock().await,
                            PutEvent::ChunkWritten { chunk_index: index },
                        )
                        .await
                        .map_err(|e| DataError::CallbackError(e.to_string()))?
                        {
                            warn!(
                                "Public update/store for '{}' cancelled after chunk {} write.",
                                name, index
                            );
                            if first_error.is_none() {
                                first_error = Some(DataError::OperationCancelled);
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                        break;
                    }
                }
            }

            if let Some(err) = first_error {
                error!(
                    "Public update/store for '{}' failed during chunk upload: {}",
                    name, err
                );
                return Err(err);
            }

            if completed_count != total_chunks {
                error!(
                    "Public update/store for '{}': Mismatch in completed chunks ({} vs {}). Internal error.",
                    name, completed_count, total_chunks
                );
                return Err(DataError::InternalError(
                    "Mismatch in completed chunk count".to_string(),
                ));
            }

            chunk_addresses = chunk_addresses_results
                .into_iter()
                .map(|opt| opt.unwrap())
                .collect();
            debug!(
                "Finished uploading {} data chunks for {}",
                total_chunks, name
            );
        }

        // 5. Serialize new list of chunk addresses using CBOR
        let new_index_data_bytes = serde_cbor::to_vec(&chunk_addresses)
            .map(Bytes::from)
            .map_err(|e| DataError::Serialization(e.to_string()))?;

        // 6. Overwrite the existing public index scratchpad using scratchpad_update directly
        debug!(
            "Updating public index scratchpad {} with new chunk list using encoding {}.",
            public_index_address, PUBLIC_INDEX_ENCODING
        );
        // Use put_raw instead
        /* manager
        .network_adapter
        .put_raw(
            &index_sk,
            &new_index_data_bytes,
            &PadStatus::Written, // Assuming Written status for update
            PUBLIC_INDEX_ENCODING,
        )
        .await
        .map_err(|e| {
            // Log the specific error from put_raw
            error!(
                "Failed to update public index scratchpad {} using put_raw: {}",
                public_index_address, e
            );
            // Map to DataError::Network or a more specific error if appropriate
            DataError::Network(e)
        })?; */

        // Avoid scratchpad_update (via put_raw) due to suspected SDK bug.
        // Instead, fetch current counter and use scratchpad_put to overwrite.
        debug!(
            "Fetching current index scratchpad {} to get counter for update...",
            public_index_address
        );
        let current_scratchpad = manager
            .network_adapter
            .get_raw_scratchpad(&public_index_address)
            .await
            .map_err(|e| {
                error!(
                    "Failed to fetch current public index scratchpad {} for update: {}",
                    public_index_address, e
                );
                DataError::Network(e) // Or more specific error
            })?;
        let current_counter = current_scratchpad.counter();
        let new_counter = current_counter + 1;
        debug!(
            "Current counter is {}, updating with counter {}",
            current_counter, new_counter
        );

        let updated_index_scratchpad = create_public_scratchpad(
            &index_sk,
            PUBLIC_INDEX_ENCODING,
            &new_index_data_bytes, // Serialized Vec<ScratchpadAddress>
            new_counter,
        );

        debug!(
            "Uploading updated index scratchpad {} via scratchpad_put",
            public_index_address
        );
        let payment = PaymentOption::Wallet((*manager.network_adapter.wallet()).clone());
        let (_cost, returned_addr) = manager
            .network_adapter
            .scratchpad_put(updated_index_scratchpad, payment)
            .await
            .map_err(|e| {
                error!(
                    "Failed to update public index scratchpad {} using scratchpad_put: {}",
                    public_index_address, e
                );
                DataError::Network(e)
            })?;

        // Sanity check the returned address
        if returned_addr != public_index_address {
            warn!(
                "scratchpad_put during update returned address {} but expected {}",
                returned_addr, public_index_address
            );
            // Continue anyway, but log warning
        }

        info!(
            "Successfully updated public index for '{}' at address {}",
            name,
            public_index_address // Use the actual address variable
        );

        // 7. Update metadata (size, modified time) in the MasterIndex via IndexManager
        debug!("Updating metadata in master index for '{}'", name);
        manager
            .index_manager
            .update_public_upload_metadata(name, total_size)
            .await
            .map_err(|e| {
                // Log failure but don't fail the whole operation
                warn!(
                    "Failed to update metadata in master index for '{}': {}. Update operation succeeded otherwise.",
                    name, e
                );
                DataError::Index(e) // Return the error type, but we won't propagate it
            })
            .ok(); // Discard the result, effectively ignoring the error

        // 8. Call Complete Event for Update Path
        if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
            .await
            .map_err(|e| DataError::CallbackError(e.to_string()))?
        {
            warn!(
                "Callback indicated cancellation after completion event for {}",
                name
            );
            // Don't return error, update technically succeeded
        }
        debug!(
            "update_public_op (update path) completed successfully for name: {}",
            name
        );
        // 9. Explicitly return Ok(()) for the update path
        Ok(())
    } else {
        // --- It does NOT exist as public key ---
        debug!(
            "Public key '{}' not found, checking if private key exists...",
            name
        );
        let maybe_private_info = manager
            .index_manager
            .get_key_info(name)
            .await
            .map_err(DataError::Index)?;
        if maybe_private_info.is_some() {
            error!(
                "Cannot update/create public key '{}': Name already exists as a private key.",
                name
            );
            return Err(DataError::InvalidOperation(format!(
                "Key '{}' exists as private.",
                name
            )));
        } else {
            // --- Does not exist at all: Perform CREATE using helper ---
            debug!(
                "Key '{}' does not exist. Creating new public upload (due to --force)...",
                name
            );

            // Call the helper function from store_public
            let _new_address = upload_public_data_and_create_index(
                manager.network_adapter.clone(),
                manager.index_manager.clone(),
                name.to_string(), // Helper takes ownership
                data_bytes,
                callback_arc.clone(), // Clone Arc for helper
            )
            .await?;
            // Helper function already calls PutEvent::Complete
            debug!(
                "update_public_op (create path) completed successfully for name: {}",
                name
            );
            // Uncommented and ensure Ok(()) is returned correctly
            Ok(())
        }
    }
}
