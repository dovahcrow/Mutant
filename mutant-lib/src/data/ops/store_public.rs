use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::index::structure::PublicUploadMetadata;
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use crate::network::AutonomiNetworkAdapter;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Stores data publicly (unencrypted) under a specified name using parallel chunk uploads.
pub(crate) async fn store_public_op(
    data_manager: &DefaultDataManager,
    name: String,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, DataError> {
    info!("DataOps: Starting store_public_op for name '{}'", name);
    let callback_arc = Arc::new(Mutex::new(callback));
    let _data_size = data_bytes.len();

    // 1. Generate Key
    let public_sk = SecretKey::random();
    debug!("Generated temporary secret key for public upload {}", name);

    // 2. Check for Name Collision
    {
        let index_copy = data_manager.index_manager.get_index_copy().await?;
        if index_copy.public_uploads.contains_key(&name) {
            error!("Public upload name '{}' already exists.", name);
            return Err(DataError::Index(
                crate::index::error::IndexError::PublicUploadNameExists(name),
            ));
        }
    }

    // 3. Chunk Data
    let chunk_size = data_manager.index_manager.get_scratchpad_size().await?;
    if chunk_size == 0 {
        error!("Configured scratchpad size is 0. Cannot proceed with chunking.");
        return Err(DataError::ChunkingError(
            "Invalid scratchpad size (0) configured".to_string(),
        ));
    }
    let chunks = chunk_data(data_bytes, chunk_size)?;
    let total_chunks = chunks.len();
    debug!(
        "Public data for '{}' chunked into {} pieces using size {}.",
        name, total_chunks, chunk_size
    );

    // Emit Starting event
    if !invoke_put_callback(
        &mut *callback_arc.lock().await,
        PutEvent::Starting {
            total_chunks,
            initial_written_count: 0, // Not applicable for public pure uploads
            initial_confirmed_count: 0, // Not applicable for public pure uploads
        },
    )
    .await
    .map_err(|e| DataError::CallbackError(e.to_string()))?
    {
        warn!("Public store for '{}' cancelled at start.", name);
        return Err(DataError::OperationCancelled);
    }

    // Declare chunk_addresses here
    let mut chunk_addresses: Vec<ScratchpadAddress>;

    if total_chunks == 0 {
        debug!(
            "Public upload '{}': No chunks to upload (empty data).",
            name
        );
        // Need to handle empty file: still create an empty index?
        // Let's upload an empty index for consistency.
        chunk_addresses = Vec::new();
    } else {
        // 4. Upload Chunks in Parallel
        let mut upload_futures = FuturesUnordered::new();
        let mut chunk_addresses_results: Vec<Option<ScratchpadAddress>> = vec![None; total_chunks];
        let public_sk_arc = Arc::new(public_sk.clone()); // Clone SK for tasks

        for (i, chunk) in chunks.into_iter().enumerate() {
            let network_adapter_clone = Arc::clone(&data_manager.network_adapter);
            let sk_clone = Arc::clone(&public_sk_arc);
            let chunk_bytes = Bytes::from(chunk); // Convert Vec<u8> to Bytes

            upload_futures.push(async move {
                trace!("Starting upload task for public chunk {}", i);
                let chunk_scratchpad = create_public_scratchpad(
                    &sk_clone,
                    PUBLIC_DATA_ENCODING,
                    &chunk_bytes,
                    0, // Use counter 0 for initial upload
                );
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

        // Process upload results
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

                    // Emit ChunkWritten event
                    if !invoke_put_callback(
                        &mut *callback_arc.lock().await,
                        PutEvent::ChunkWritten { chunk_index: index },
                    )
                    .await
                    .map_err(|e| DataError::CallbackError(e.to_string()))?
                    {
                        warn!(
                            "Public store for '{}' cancelled after chunk {} write.",
                            name, index
                        );
                        if first_error.is_none() {
                            first_error = Some(DataError::OperationCancelled);
                        }
                        // Don't return yet, let other futures finish?
                        // For consistency with store_op, let's break and report first error.
                        break;
                    }
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                    // Break on first error to stop processing
                    break;
                }
            }
        }

        // Check if loop was exited due to error
        if let Some(err) = first_error {
            error!(
                "Public store for '{}' failed during chunk upload: {}",
                name, err
            );
            // Drain remaining futures? store_op doesn't explicitly do this, relies on drop?
            return Err(err);
        }

        // Verify all chunks completed successfully
        if completed_count != total_chunks {
            error!(
                "Public store for '{}': Mismatch in completed chunks ({} vs {}). Internal error.",
                name, completed_count, total_chunks
            );
            return Err(DataError::InternalError(
                "Mismatch in completed chunk count".to_string(),
            ));
        }

        // Convert Vec<Option<ScratchpadAddress>> to Vec<ScratchpadAddress>
        chunk_addresses = chunk_addresses_results
            .into_iter()
            .map(|opt| opt.unwrap())
            .collect();

        debug!(
            "Finished uploading {} data chunks in parallel for {}",
            total_chunks, name
        );
    }

    // 5. Create and Upload Index Scratchpad (Sequentially after chunks)
    let index_data_bytes = serde_cbor::to_vec(&chunk_addresses)
        .map(Bytes::from)
        .map_err(|e| DataError::Serialization(e.to_string()))?;

    let index_scratchpad = create_public_scratchpad(
        &public_sk,
        PUBLIC_INDEX_ENCODING,
        &index_data_bytes,
        0, // Use counter 0 for initial upload
    );
    let public_index_address = *index_scratchpad.address();
    trace!(
        "Uploading public index scratchpad ({}) for {}",
        public_index_address,
        name
    );

    let payment = PaymentOption::Wallet((*data_manager.network_adapter.wallet()).clone());
    match data_manager
        .network_adapter
        .scratchpad_put(index_scratchpad, payment)
        .await
    {
        Ok((_cost, addr)) => {
            debug!("Successfully uploaded public index {} for {}", addr, name);
            if addr != public_index_address {
                error!(
                    "Returned public index address {} does not match calculated address {} for {}",
                    addr, public_index_address, name
                );
                return Err(DataError::InternalError(
                    "Public index address mismatch after upload".to_string(),
                ));
            }
        }
        Err(e) => {
            error!(
                "Failed to upload public index scratchpad ({}) for {}: {}",
                public_index_address, name, e
            );
            return Err(DataError::Network(e));
        }
    }

    // 6. Update Master Index
    debug!("Updating master index for public upload {}", name);
    let metadata = PublicUploadMetadata {
        address: public_index_address,
        key_bytes: public_sk.to_bytes().to_vec(),
    };

    // Call the dedicated IndexManager method
    data_manager
        .index_manager
        .insert_public_upload_metadata(name.clone(), metadata)
        .await?;

    info!(
        "Successfully stored public data '{}' at index {}",
        name, public_index_address
    );

    // Final Complete event
    if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
    {
        warn!("Public store for '{}' cancelled after completion.", name);
        // Don't return error here, operation technically succeeded.
    }

    Ok(public_index_address)
}
