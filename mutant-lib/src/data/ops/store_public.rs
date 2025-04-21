use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::{PUBLIC_DATA_ENCODING, PUBLIC_INDEX_ENCODING};
use crate::index::error::IndexError;
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::{IndexEntry, PadInfo, PadStatus, PublicUploadInfo};
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use crate::network::error::NetworkError;
use crate::network::AutonomiNetworkAdapter;
use crate::pad_lifecycle::PadOrigin;
use autonomi::client::payment::PaymentOption;
use autonomi::client::quote::DataTypes;
use autonomi::{Bytes, ScratchpadAddress, SecretKey, XorName};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
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
        if let Some(entry) = index_copy.index.get(&name) {
            // Get the entry if it exists
            let error_to_return = match entry {
                IndexEntry::PublicUpload(_) => {
                    error!(
                        "Public upload name '{}' collides with an existing public upload.",
                        name
                    );
                    IndexError::PublicUploadNameExists(name.clone()) // Use specific error
                }
                IndexEntry::PrivateKey(_) => {
                    error!(
                        "Public upload name '{}' collides with an existing private key.",
                        name
                    );
                    IndexError::KeyExists(name.clone()) // Use generic error for private keys
                }
            };
            return Err(DataError::Index(error_to_return));
        }
        // No collision found, proceed
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
    const INITIAL_COUNTER: u64 = 0;

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

    let mut data_pad_infos: Vec<PadInfo> = Vec::new();

    if total_chunks == 0 {
        debug!(
            "Public upload '{}': No chunks to upload (empty data).",
            name
        );
        // No data pads needed
    } else {
        // Generate keys and addresses (no sizes needed now)
        let data_pad_keys_addrs: Vec<(SecretKey, ScratchpadAddress)> = (0..total_chunks)
            .map(|_| {
                let sk = SecretKey::random();
                let addr = ScratchpadAddress::new(sk.public_key());
                (sk, addr)
            })
            .collect();

        let mut upload_futures = FuturesUnordered::new();
        let mut chunk_results: Vec<Option<PadInfo>> = vec![None; total_chunks];

        // Iterate using pre-generated keys/addresses and original chunks
        for (i, ((chunk_sk, chunk_addr), chunk_data)) in data_pad_keys_addrs
            .into_iter()
            .zip(chunks.into_iter())
            .enumerate()
        {
            let adapter_clone = Arc::clone(&network_adapter);
            let chunk_sk_bytes = chunk_sk.to_bytes().to_vec();
            let chunk_bytes = Bytes::from(chunk_data);

            upload_futures.push(async move {
                trace!(
                    "Starting upload task for public chunk {} -> {}",
                    i,
                    chunk_addr
                );
                let chunk_scratchpad = create_public_scratchpad(
                    &chunk_sk,
                    PUBLIC_DATA_ENCODING,
                    &chunk_bytes,
                    INITIAL_COUNTER,
                );
                let payment = PaymentOption::Wallet((*adapter_clone.wallet()).clone());

                match adapter_clone
                    .scratchpad_put(chunk_scratchpad, payment)
                    .await
                {
                    Ok((_cost, returned_addr)) => {
                        // Original put result (cost, addr)
                        if returned_addr != chunk_addr {
                            warn!(
                                "Address mismatch for chunk {}: expected {}, got {}",
                                i, chunk_addr, returned_addr
                            );
                            // Use returned_addr anyway
                        }
                        trace!(
                            "Put successful for public chunk {} -> {}. Fetching quote...",
                            i,
                            returned_addr
                        );

                        // --- Get Quote and Receipt ---
                        let quote_result = adapter_clone
                            .get_store_quotes(
                                DataTypes::Scratchpad,
                                std::iter::once((
                                    XorName::from(returned_addr.xorname()), // Use returned address XorName
                                    chunk_bytes.len(),                      // Use actual chunk size
                                )),
                            )
                            .await;

                        match quote_result {
                            Ok(quotes) => {
                                let receipt =
                                    autonomi::client::payment::receipt_from_store_quotes(quotes);
                                trace!(
                                    "Quote/Receipt obtained successfully for chunk {} -> {}",
                                    i,
                                    returned_addr
                                );
                                let pad_info = PadInfo {
                                    address: returned_addr,
                                    chunk_index: i,
                                    status: PadStatus::Written,
                                    origin: PadOrigin::Generated,
                                    needs_reverification: false,
                                    last_known_counter: INITIAL_COUNTER,
                                    sk_bytes: chunk_sk_bytes,
                                    receipt: Some(receipt), // Store the generated receipt
                                };
                                Ok((i, pad_info))
                            }
                            Err(e) => {
                                error!(
                                    "Failed to get store quote for chunk {} -> {}: {}",
                                    i, returned_addr, e
                                );
                                // Map CostError or other errors to DataError::Network or a new variant
                                Err((i, DataError::Network(e.into())))
                            }
                        }
                        // --- End Get Quote and Receipt ---
                    }
                    Err(e) => {
                        error!(
                            "Put task failed for public chunk {} (addr: {}): {}",
                            i, chunk_addr, e
                        );
                        Err((i, DataError::Network(e))) // Keep original put error mapping
                    }
                }
            });
        }

        // Process upload results
        let mut first_error: Option<DataError> = None;
        let mut completed_count = 0;

        while let Some(result) = upload_futures.next().await {
            match result {
                Ok((index, pad_info)) => {
                    debug!(
                        "Successfully uploaded public chunk {} ({}) for {}",
                        index, pad_info.address, name
                    );
                    chunk_results[index] = Some(pad_info); // Store PadInfo in the correct slot
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
                Err((index, e)) => {
                    // Error case now returns index
                    error!("Error processing chunk {}: {}", index, e);
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

        data_pad_infos = chunk_results
            .into_iter()
            .map(|opt| opt.expect("Internal error: Missing PadInfo after successful check"))
            .collect();

        debug!(
            "Finished uploading {} data chunks in parallel for {}",
            total_chunks, name
        );
    }

    // 5. Create and Upload Index Scratchpad
    let index_sk = SecretKey::random();
    let index_sk_bytes = index_sk.to_bytes().to_vec();
    let public_index_address = ScratchpadAddress::new(index_sk.public_key());

    // Serialize only the addresses for the index content
    let index_content_addresses: Vec<ScratchpadAddress> =
        data_pad_infos.iter().map(|p| p.address).collect();
    let index_data_bytes = serde_cbor::to_vec(&index_content_addresses)
        .map(Bytes::from)
        .map_err(|e| DataError::Serialization(e.to_string()))?;
    let _index_data_size = index_data_bytes.len();

    let index_scratchpad = create_public_scratchpad(
        &index_sk,
        PUBLIC_INDEX_ENCODING,
        &index_data_bytes,
        INITIAL_COUNTER,
    );

    trace!(
        "Uploading public index scratchpad ({}) for {}",
        public_index_address,
        name
    );

    let payment = PaymentOption::Wallet((*network_adapter.wallet()).clone());
    let index_pad_info_result: Result<PadInfo, DataError> = match network_adapter
        .scratchpad_put(index_scratchpad, payment)
        .await
    {
        Ok((_cost, returned_addr)) => {
            // Original put result
            debug!(
                "Successfully initiated upload for public index {} for {}. Fetching quote...",
                returned_addr, name
            );
            if returned_addr != public_index_address {
                warn!(
                    "Returned public index address {} does not match calculated address {} for {}",
                    returned_addr, public_index_address, name
                );
                // Proceed with returned_addr
            }

            // --- Get Quote and Receipt for Index ---
            let index_quote_result = network_adapter
                .get_store_quotes(
                    DataTypes::Scratchpad,
                    std::iter::once((
                        XorName::from(returned_addr.xorname()), // Use returned address XorName
                        index_data_bytes.len(),                 // Use actual index size
                    )),
                )
                .await;

            let index_receipt_result = match index_quote_result {
                Ok(quotes) => Ok(autonomi::client::payment::receipt_from_store_quotes(quotes)),
                Err(e) => {
                    error!(
                        "Failed to get store quote for index pad {}: {}",
                        returned_addr, e
                    );
                    // Map CostError or other errors to DataError::Network or a new variant
                    Err(DataError::Network(e.into()))
                }
            };

            let index_receipt = index_receipt_result?; // Propagate error if quote/receipt failed
            trace!(
                "Quote/Receipt obtained successfully for index pad {}",
                returned_addr
            );
            // --- End Get Quote and Receipt for Index ---

            // --- Verification Step ---
            let mut last_error: Option<DataError> = None;
            for attempt in 0..PUBLIC_CONFIRMATION_RETRY_LIMIT {
                trace!(
                    "Verification attempt {} for public index scratchpad {}",
                    attempt + 1,
                    returned_addr // Verify the returned address
                );
                sleep(PUBLIC_CONFIRMATION_RETRY_DELAY).await; // Wait before fetching

                match network_adapter.get_raw_scratchpad(&returned_addr).await {
                    Ok(fetched_pad) => {
                        // Check encoding
                        if fetched_pad.data_encoding() == PUBLIC_INDEX_ENCODING {
                            // Check counter
                            if fetched_pad.counter() == INITIAL_COUNTER {
                                info!(
                                     "Public index scratchpad {} verified successfully (Encoding: {}, Counter: {}).",
                                     returned_addr, PUBLIC_INDEX_ENCODING, INITIAL_COUNTER
                                 );
                                last_error = None; // Clear previous errors, verification passed
                                break; // Verification successful, exit retry loop
                            } else {
                                warn!(
                                     "Verification attempt {}: Public index {} has unexpected counter {} (expected {}). Retrying...",
                                     attempt + 1,
                                     returned_addr,
                                     fetched_pad.counter(),
                                     INITIAL_COUNTER
                                 );
                                last_error = Some(DataError::InconsistentState(format!(
                                    "Public index {} has unexpected counter {} after upload",
                                    returned_addr,
                                    fetched_pad.counter()
                                )));
                            }
                        } else {
                            warn!(
                                 "Verification attempt {}: Public index {} has incorrect encoding {} (expected {}). Retrying...",
                                 attempt + 1,
                                 returned_addr,
                                 fetched_pad.data_encoding(),
                                 PUBLIC_INDEX_ENCODING
                             );
                            last_error = Some(DataError::InvalidPublicIndexEncoding(
                                fetched_pad.data_encoding(),
                            ));
                        }
                    }
                    Err(NetworkError::InternalError(e_str))
                        if e_str.to_lowercase().contains("not found")
                            || e_str.to_lowercase().contains("does not exist") =>
                    {
                        warn!(
                              "Verification attempt {}: Public index {} not found (via InternalError: {}). Retrying...",
                               attempt + 1,
                               returned_addr,
                               e_str
                           );
                        last_error = Some(DataError::PublicScratchpadNotFound(returned_addr));
                    }
                    Err(e) => {
                        warn!(
                             "Verification attempt {}: Unexpected error fetching public index {}: {}. Retrying...",
                             attempt + 1,
                             returned_addr,
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
                    returned_addr, name, PUBLIC_CONFIRMATION_RETRY_LIMIT, final_error
                );
                Err(final_error) // Return error from verification failure
            } else {
                // Verification successful, create PadInfo
                Ok(PadInfo {
                    address: returned_addr,
                    chunk_index: 0,
                    status: PadStatus::Written,
                    origin: PadOrigin::Generated,
                    needs_reverification: false,
                    last_known_counter: INITIAL_COUNTER,
                    sk_bytes: index_sk_bytes,
                    receipt: Some(index_receipt), // Store the generated receipt
                })
            }
            // --- End Verification Step ---
        }
        Err(e) => {
            error!(
                "Failed to upload public index scratchpad ({}) for {}: {}",
                public_index_address,
                name,
                e // Log calculated address on error
            );
            Err(DataError::Network(e)) // Return error from upload failure
        }
    };

    let index_pad_info = index_pad_info_result?;

    // 6. Update Master Index
    debug!("Updating master index for public upload {}", name);
    let public_upload_info = PublicUploadInfo {
        index_pad: index_pad_info, // Store the full PadInfo for the index
        size: data_size,
        modified: Utc::now(),
        data_pads: data_pad_infos, // Store the Vec<PadInfo> for data pads
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
        name,
        public_index_address // Log original calculated address for user info
    );

    // Final Complete event
    if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
    {
        warn!("Public store for '{}' cancelled after completion.", name);
        // Don't return error here, operation technically succeeded.
    }

    Ok(public_index_address) // Return the calculated (intended) index address
}

// Remove placeholder implementation from here, add to adapter
