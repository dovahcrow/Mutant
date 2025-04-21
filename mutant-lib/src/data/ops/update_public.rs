use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::index::structure::PadInfo;
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) async fn update_public_op(
    manager: &DefaultDataManager,
    name: &str,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<(), DataError> {
    info!("DataOps: Starting update_public_op for name '{}'", name);
    let callback_arc = Arc::new(Mutex::new(callback));
    let new_data_size = data_bytes.len();

    // 1. Get existing public info
    debug!("Fetching existing public upload info for '{}'...", name);
    let mut public_info = manager
        .index_manager
        .get_public_upload_info(name)
        .await
        .map_err(DataError::Index)?
        .ok_or_else(|| {
            error!("Public upload '{}' not found for update.", name);
            // Before returning NotFound, check if it exists as a private key
            // This check requires an async block or separate function
            // For now, assume it doesn't exist if get_public_upload_info returned None
            DataError::KeyNotFound(name.to_string())
        })?;

    // Retrieve index pad details for later update
    let index_pad_info = public_info.index_pad.clone(); // Clone needed info

    // 2. Chunk New Data
    let chunk_size = manager.index_manager.get_scratchpad_size().await?;
    if chunk_size == 0 {
        error!("Configured scratchpad size is 0. Cannot proceed with chunking.");
        return Err(DataError::ChunkingError(
            "Invalid scratchpad size (0) configured".to_string(),
        ));
    }
    let new_chunks = chunk_data(data_bytes, chunk_size)?;
    let new_total_chunks = new_chunks.len();
    let old_total_chunks = public_info.data_pads.len();
    debug!(
        "Public data for '{}' chunked into {} pieces (was {}).",
        name, new_total_chunks, old_total_chunks
    );

    // --- Handle Chunk Count Mismatch (Scenario B - Simplified/Error for now) ---
    if new_total_chunks != old_total_chunks {
        // For now, we don't support changing the number of chunks during update.
        // This requires deleting old pads and creating new ones, re-introducing payment issues.
        error!(
            "Update for '{}' failed: Number of chunks changed from {} to {}. This is not yet supported.",
            name, old_total_chunks, new_total_chunks
        );
        // Ensure this returns immediately
        return Err(DataError::InvalidOperation(
            "Updating with a different number of chunks is not supported yet.".to_string(),
        ));
        // Removed TODO comment as return is sufficient
    }
    // --- End Chunk Count Mismatch Handling ---

    // If chunk count is 0 (both old and new must be 0 here)
    if new_total_chunks == 0 {
        debug!(
            "Public update '{}': No data chunks to update (empty data).",
            name
        );
        // Skip data pad updates, proceed directly to index pad update.
    } else {
        // --- Update Data Pads (Scenario A) ---
        debug!(
            "Updating {} existing data pads in parallel...",
            new_total_chunks
        );

        // 3. Emit Starting event (using new chunk count, which is same as old)
        if !invoke_put_callback(
            &mut *callback_arc.lock().await,
            PutEvent::Starting {
                total_chunks: new_total_chunks,
                initial_written_count: 0,   // Not applicable
                initial_confirmed_count: 0, // Not applicable
            },
        )
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
        {
            warn!("Public update for '{}' cancelled at start.", name);
            return Err(DataError::OperationCancelled);
        }

        let mut update_futures = FuturesUnordered::new();
        // Keep track of updated PadInfo structs
        let mut updated_data_pad_infos: Vec<Option<PadInfo>> = vec![None; new_total_chunks];

        // Iterate through existing pads and new chunks together
        for (i, (mut existing_pad_info, new_chunk_data)) in public_info
            .data_pads // Use existing pads
            .into_iter() // Note: This consumes public_info.data_pads temporarily
            .zip(new_chunks.into_iter())
            .enumerate()
        {
            let network_adapter_clone = Arc::clone(&manager.network_adapter);
            let chunk_bytes = Bytes::from(new_chunk_data);
            let _callback_arc_clone = Arc::clone(&callback_arc); // Clone for async block

            update_futures.push(async move {
                let pad_address = existing_pad_info.address; // Store for error messages
                trace!(
                    "Starting update task for data chunk {} on pad {}",
                    i,
                    pad_address
                );

                // 1. Fetch current counter (required for scratchpad_put)
                let current_pad = match network_adapter_clone.get_raw_scratchpad(&pad_address).await {
                    Ok(pad) => pad,
                    Err(e) => {
                        error!("Failed to fetch current data pad {} to get counter for update: {}", pad_address, e);
                        return Err((i, DataError::Network(e)));
                    }
                };
                let current_counter = current_pad.counter();
                let new_counter = current_counter + 1;
                trace!("Data pad {} current counter: {}, new counter: {}", pad_address, current_counter, new_counter);


                // 2. Prepare Scratchpad object for put
                let owner_key = match existing_pad_info.sk_bytes.as_slice().try_into() {
                    Ok(sk_array) => SecretKey::from_bytes(sk_array).map_err(|_| {
                        DataError::InternalError(format!(
                            "Invalid secret key format for pad {}",
                            pad_address
                        ))
                    }),
                    Err(_) => Err(DataError::InternalError(format!(
                        "Invalid secret key length ({}) for pad {}",
                        existing_pad_info.sk_bytes.len(),
                        pad_address
                    ))),
                }
                .map_err(|e| (i, e))?; // Map error to (usize, DataError) and propagate


                let chunk_scratchpad = create_public_scratchpad(
                    &owner_key,
                    PUBLIC_DATA_ENCODING,
                    &chunk_bytes,
                    new_counter,
                );


                // 3. Determine Payment Option (using wallet)
                let payment_option = PaymentOption::Wallet((*network_adapter_clone.wallet()).clone());
                /* match existing_pad_info.receipt.clone() { // Clone receipt if needed
                    Some(receipt) => PaymentOption::Receipt(receipt),
                    None => {
                        error!("Missing payment receipt for existing data pad {} during update.", pad_address);
                        return Err((i, DataError::MissingReceipt(pad_address)));
                    }
                }; */

                // 4. Call scratchpad_put
                trace!("Calling scratchpad_put for data chunk {} on pad {} with counter {}", i, pad_address, new_counter);
                match network_adapter_clone
                    .scratchpad_put(chunk_scratchpad, payment_option)
                    .await
                {
                    Ok((_cost, returned_addr)) => { // Destructure cost and address
                        trace!(
                            "Put task successful for data chunk {} on pad {} (returned {})",
                            i,
                            pad_address,
                            returned_addr
                        );
                        if returned_addr != pad_address {
                             warn!(
                                "scratchpad_put during data update for chunk {} returned address {} but expected {}",
                                i, returned_addr, pad_address
                            );
                            // Decide if this is fatal? For now, just warn and proceed.
                        }
                        // Update counter in the pad info
                        existing_pad_info.last_known_counter = new_counter;
                        Ok((i, existing_pad_info)) // Return index and updated PadInfo
                    }
                    Err(e) => {
                        error!(
                            "Put task failed for data chunk {} on pad {}: {}",
                            i, pad_address, e
                        );
                        Err((i, DataError::Network(e))) // Return index and error
                    }
                }
            });
        }

        // Process update results
        let mut first_error: Option<DataError> = None;
        let mut completed_count = 0;

        while let Some(result) = update_futures.next().await {
            match result {
                Ok((index, updated_pad_info)) => {
                    debug!(
                        "Successfully updated data chunk {} on pad {}",
                        index, updated_pad_info.address
                    );
                    updated_data_pad_infos[index] = Some(updated_pad_info); // Store updated PadInfo
                    completed_count += 1;

                    // Emit ChunkWritten event
                    if !invoke_put_callback(
                        &mut *callback_arc.lock().await,
                        PutEvent::ChunkWritten { chunk_index: index },
                    )
                    .await
                    .map_err(|e| DataError::CallbackError(e.to_string()))?
                    {
                        warn!(
                            "Public update for '{}' cancelled after chunk {} update.",
                            name, index
                        );
                        if first_error.is_none() {
                            first_error = Some(DataError::OperationCancelled);
                        }
                        break;
                    }
                }
                Err((index, e)) => {
                    error!("Error updating chunk {}: {}", index, e);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                    break;
                }
            }
        }

        // Handle errors and completion check
        if let Some(err) = first_error {
            error!(
                "Public update for '{}' failed during data chunk update: {}",
                name, err
            );
            // Reconstruct public_info.data_pads before returning error
            // This is tricky because the original was moved. We need to re-fetch or rethink ownership.
            // For now, the state might be inconsistent if an error occurs mid-way.
            // Consider collecting results into a Vec first, then updating public_info if all succeed.
            return Err(err);
        }

        if completed_count != new_total_chunks {
            error!(
                 "Public update for '{}': Mismatch in completed chunk updates ({} vs {}). Internal error.",
                 name, completed_count, new_total_chunks
             );
            // Similar state issue as above if we error out here.
            return Err(DataError::InternalError(
                "Mismatch in completed chunk update count".to_string(),
            ));
        }

        // Update the data_pads in public_info with the results
        // Ensure we only collect if successful completion
        public_info.data_pads = updated_data_pad_infos
            .into_iter()
            .map(|opt| {
                opt.expect("Internal error: Missing updated PadInfo after successful completion")
            })
            .collect();

        debug!("Finished updating {} data pads.", new_total_chunks);
    }
    // --- End Update Data Pads ---

    // 4. Update Index Pad (Reverting to scratchpad_put via fetch counter)
    debug!(
        "Updating index pad {} using scratchpad_put (fetch counter method)...",
        index_pad_info.address
    );

    // Fetch current counter
    let current_index_pad = manager
        .network_adapter
        .get_raw_scratchpad(&index_pad_info.address)
        .await
        .map_err(|e| {
            error!(
                "Failed to fetch current index pad {} to get counter for update: {}",
                index_pad_info.address, e
            );
            DataError::Network(e)
        })?;
    let current_counter = current_index_pad.counter();
    let new_counter = current_counter + 1;
    debug!(
        "Current index counter is {}, updating with counter {}",
        current_counter, new_counter
    );

    let index_owner_key =
        SecretKey::from_bytes(index_pad_info.sk_bytes.as_slice().try_into().map_err(|_| {
            DataError::InternalError(format!(
                "Invalid secret key length ({}) for index pad {}",
                index_pad_info.sk_bytes.len(),
                index_pad_info.address
            ))
        })?)
        .map_err(|_| {
            DataError::InternalError(format!(
                "Invalid secret key format for index pad {}",
                index_pad_info.address
            ))
        })?;

    // Serialize the list of potentially updated data pad addresses
    let updated_index_content_addresses: Vec<ScratchpadAddress> =
        public_info.data_pads.iter().map(|p| p.address).collect();
    let new_index_data_bytes = serde_cbor::to_vec(&updated_index_content_addresses)
        .map(Bytes::from)
        .map_err(|e| DataError::Serialization(e.to_string()))?;

    // Create a new scratchpad object with updated data and new counter
    let updated_index_scratchpad = create_public_scratchpad(
        &index_owner_key,
        PUBLIC_INDEX_ENCODING,
        &new_index_data_bytes,
        new_counter, // Use the incremented counter
    );

    // Determine Payment Option for Index Pad (using wallet)
    let index_payment_option = PaymentOption::Wallet((*manager.network_adapter.wallet()).clone());
    /* match index_pad_info.receipt.clone() { // Clone receipt if needed
        Some(receipt) => PaymentOption::Receipt(receipt),
        None => {
            error!(
                "Missing payment receipt for index pad {} during update.",
                index_pad_info.address
            );
            return Err(DataError::MissingReceipt(index_pad_info.address));
        }
    }; */

    // Call scratchpad_put to overwrite
    trace!(
        "Calling scratchpad_put for index pad {} with counter {}",
        index_pad_info.address,
        new_counter
    );
    let (_cost, returned_addr) = manager // Destructure cost and address
        .network_adapter
        .scratchpad_put(updated_index_scratchpad, index_payment_option) // Use receipt payment
        .await
        .map_err(|e| {
            error!(
                "Failed to update index pad {} using scratchpad_put: {}",
                index_pad_info.address, e
            );
            DataError::Network(e)
        })?;

    // Sanity check address
    if returned_addr != index_pad_info.address {
        warn!(
            "scratchpad_put during index update returned address {} but expected {}",
            returned_addr, index_pad_info.address
        );
        // Continue anyway?
    }

    // Update counter in the index PadInfo stored in public_info
    public_info.index_pad.last_known_counter = new_counter; // Use the counter we just put

    info!(
        "Successfully updated index pad {} for '{}' via scratchpad_put",
        index_pad_info.address, name
    );

    // 5. Update metadata in MasterIndex
    debug!("Updating public upload info in master index for '{}'", name);
    // Update size and modified time
    public_info.size = new_data_size;
    public_info.modified = Utc::now();
    // data_pads and index_pad counters were updated in place above

    manager
        .index_manager
        .replace_public_upload_info(name, public_info) // Pass the modified public_info
        .await
        .map_err(|e| {
            error!(
                "Failed to replace public upload info in master index for '{}': {}",
                name, e
            );
            DataError::Index(e)
        })?;

    // 6. Call Complete Event
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

    debug!("update_public_op completed successfully for name: {}", name);
    Ok(())
}
