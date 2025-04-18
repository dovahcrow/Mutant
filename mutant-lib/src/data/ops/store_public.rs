use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::PublicUploadInfo;
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use crate::network::error::NetworkError;
use crate::network::AutonomiNetworkAdapter;
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;
use tokio::time::sleep;

// Define retry constants locally for public verification
const PUBLIC_CONFIRMATION_RETRY_LIMIT: u32 = 3;
const PUBLIC_CONFIRMATION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Stores data publicly (unencrypted) under a specified name using parallel chunk uploads.
pub(crate) async fn store_public_op(
    data_manager: &DefaultDataManager,
    name: String,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, DataError> {
    info!("DataOps: Starting store_public_op for name '{}'", name);
    let callback_arc = Arc::new(Mutex::new(callback));

    // 1. Check for Name Collision
    {
        let index_copy = data_manager.index_manager.get_index_copy().await?;
        if index_copy.index.contains_key(&name) {
            error!(
                "Public upload name '{}' collides with an existing entry.",
                name
            );
            return Err(DataError::Index(
                crate::index::error::IndexError::KeyExists(name),
            ));
        }
    }

    // 2. Call the helper function to perform the actual work
    upload_public_data_and_create_index(
        data_manager.network_adapter.clone(),
        data_manager.index_manager.clone(),
        name,
        data_bytes,
        callback_arc,
    )
    .await
}

// Helper function to handle chunking, uploading, index creation, and index manager update
pub(crate) async fn upload_public_data_and_create_index(
    network_adapter: Arc<AutonomiNetworkAdapter>,
    index_manager: Arc<DefaultIndexManager>,
    name: String,
    data_bytes: &[u8],
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<ScratchpadAddress, DataError> {
    let data_size = data_bytes.len();

    // Chunk Data
    let chunk_size = index_manager.get_scratchpad_size().await?;
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

    let chunk_addresses: Vec<ScratchpadAddress>;

    if total_chunks == 0 {
        debug!(
            "Public upload '{}': No chunks to upload (empty data).",
            name
        );
        // Need to handle empty file: still create an empty index?
        // Let's upload an empty index for consistency.
        chunk_addresses = Vec::new();
    } else {
        // Upload Chunks in Parallel
        let mut upload_futures = FuturesUnordered::new();
        let mut chunk_addresses_results: Vec<Option<ScratchpadAddress>> = vec![None; total_chunks];

        for (i, chunk) in chunks.into_iter().enumerate() {
            let adapter_clone = Arc::clone(&network_adapter);
            let chunk_sk = SecretKey::random();
            let chunk_bytes = Bytes::from(chunk);

            upload_futures.push(async move {
                trace!("Starting upload task for public chunk {}", i);
                let chunk_scratchpad = create_public_scratchpad(
                    &chunk_sk,
                    PUBLIC_DATA_ENCODING,
                    &chunk_bytes,
                    0, // Use counter 0 for initial upload
                );
                let chunk_addr = *chunk_scratchpad.address();
                let payment = PaymentOption::Wallet((*adapter_clone.wallet()).clone());

                match adapter_clone
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
    let index_sk = SecretKey::random();

    let index_data_bytes = serde_cbor::to_vec(&chunk_addresses)
        .map(Bytes::from)
        .map_err(|e| DataError::Serialization(e.to_string()))?;

    let index_scratchpad = create_public_scratchpad(
        &index_sk,
        PUBLIC_INDEX_ENCODING,
        &index_data_bytes,
        0, // Use counter 0 for initial upload
    );
    let public_index_address = ScratchpadAddress::new(index_sk.public_key());
    trace!(
        "Uploading public index scratchpad ({}) for {}",
        public_index_address,
        name
    );

    let payment = PaymentOption::Wallet((*network_adapter.wallet()).clone());
    match network_adapter
        .scratchpad_put(index_scratchpad, payment)
        .await
    {
        Ok((_cost, addr)) => {
            debug!(
                "Successfully initiated upload for public index {} for {}",
                addr, name
            );
            if addr != public_index_address {
                error!(
                    "Returned public index address {} does not match calculated address {} for {}",
                    addr, public_index_address, name
                );
                // Even if addresses mismatch, the SDK might have done something unexpected.
                // Let's try verifying the returned address 'addr' before failing hard.
                // If verification passes on 'addr', maybe log a warning and continue?
                // For now, stick to original strict check.
                return Err(DataError::InternalError(
                    "Public index address mismatch immediately after upload".to_string(),
                ));
            }
            // --- Verification Step Added ---
            let mut last_error: Option<DataError> = None;
            for attempt in 0..PUBLIC_CONFIRMATION_RETRY_LIMIT {
                trace!(
                    "Verification attempt {} for public index scratchpad {}",
                    attempt + 1,
                    public_index_address
                );
                sleep(PUBLIC_CONFIRMATION_RETRY_DELAY).await; // Wait before fetching

                match network_adapter
                    .get_raw_scratchpad(&public_index_address)
                    .await
                {
                    Ok(fetched_pad) => {
                        // Check encoding
                        if fetched_pad.data_encoding() == PUBLIC_INDEX_ENCODING {
                            // Check counter (should be 0 for initial upload)
                            if fetched_pad.counter() == 0 {
                                info!(
                                    "Public index scratchpad {} verified successfully (Encoding: {}, Counter: 0).",
                                    public_index_address, PUBLIC_INDEX_ENCODING
                                );
                                last_error = None; // Clear previous errors, verification passed
                                break; // Verification successful, exit retry loop
                            } else {
                                warn!(
                                    "Verification attempt {}: Public index {} has unexpected counter {} (expected 0). Retrying...",
                                    attempt + 1,
                                    public_index_address,
                                    fetched_pad.counter()
                                );
                                last_error = Some(DataError::InconsistentState(format!(
                                    "Public index {} has unexpected counter {} after upload",
                                    public_index_address,
                                    fetched_pad.counter()
                                )));
                            }
                        } else {
                            warn!(
                                "Verification attempt {}: Public index {} has incorrect encoding {} (expected {}). Retrying...",
                                attempt + 1,
                                public_index_address,
                                fetched_pad.data_encoding(),
                                PUBLIC_INDEX_ENCODING
                            );
                            last_error = Some(DataError::InvalidPublicIndexEncoding(
                                fetched_pad.data_encoding(),
                            ));
                        }
                    }
                    Err(NetworkError::InternalError(e_str)) => {
                        // Check if the internal error string indicates 'not found'
                        if e_str.to_lowercase().contains("not found")
                            || e_str.to_lowercase().contains("does not exist")
                        {
                            warn!(
                                 "Verification attempt {}: Public index {} not found (via InternalError: {}). Retrying...",
                                  attempt + 1,
                                  public_index_address,
                                  e_str
                              );
                            last_error =
                                Some(DataError::PublicScratchpadNotFound(public_index_address));
                        } else {
                            // Treat other internal errors as generic network errors for retry
                            warn!(
                                "Verification attempt {}: Network/Internal error fetching public index {}: {}. Retrying...",
                                attempt + 1,
                                public_index_address,
                                e_str
                            );
                            last_error =
                                Some(DataError::Network(NetworkError::InternalError(e_str)));
                            // Re-wrap error
                        }
                    }
                    Err(e) => {
                        // Catch any other NetworkError variants
                        warn!(
                            "Verification attempt {}: Unexpected error fetching public index {}: {}. Retrying...",
                            attempt + 1,
                            public_index_address,
                            e
                        );
                        last_error = Some(DataError::Network(e));
                    }
                }
            } // End retry loop

            // If loop finished and last_error is still Some, verification failed
            if let Some(final_error) = last_error {
                error!(
                    "Failed to verify public index scratchpad {} for {} after {} attempts: {}",
                    public_index_address, name, PUBLIC_CONFIRMATION_RETRY_LIMIT, final_error
                );
                return Err(final_error);
            }
            // --- End Verification Step ---
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
    let public_upload_info = PublicUploadInfo {
        address: public_index_address,
        size: data_size,
        modified: Utc::now(),
        index_secret_key_bytes: index_sk.to_bytes().to_vec(),
    };

    if let Err(e) = index_manager
        .insert_public_upload_info(name.clone(), public_upload_info)
        .await
    {
        error!(
            "Failed to insert public upload metadata for '{}' into index: {}",
            name, e
        );
        // Should we try to clean up the uploaded index scratchpad?
        // For now, return the index error.
        return Err(DataError::Index(e));
    }

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

// Helper to verify public scratchpad existence (simplified from purge logic)
async fn verify_scratchpad_exists(
    network_adapter: &Arc<AutonomiNetworkAdapter>,
    address: &ScratchpadAddress,
) -> Result<bool, NetworkError> {
    match network_adapter.get_raw_scratchpad(address).await {
        Ok(_) => Ok(true), // Found it
        Err(NetworkError::InternalError(msg)) if msg.contains("not found") => Ok(false), // Confirmed not found
        Err(e) => Err(e), // Other network error
    }
}
