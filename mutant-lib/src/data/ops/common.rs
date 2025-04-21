use crate::data::error::DataError;
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::{PadInfo, PadStatus};
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::network::adapter::create_public_scratchpad;
use crate::network::{AutonomiNetworkAdapter, NetworkError};
use autonomi::client::payment::{PaymentOption, Receipt};
use autonomi::client::quote::DataTypes;
use autonomi::{Bytes, ScratchpadAddress, SecretKey, XorName};
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

// Constants for private write/confirmation retries
pub(crate) const CONFIRMATION_RETRY_LIMIT: u32 = 3;
pub(crate) const CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(10);

// Input for private write tasks
pub(crate) struct WriteTaskInput {
    pub pad_info: PadInfo,
    pub secret_key: SecretKey,
    pub chunk_data: Bytes,
}

// --- Public Upload Task Structures & Execution ---

pub(crate) const PUBLIC_CONFIRMATION_RETRY_LIMIT: u32 = 3;
pub(crate) const PUBLIC_CONFIRMATION_RETRY_DELAY: Duration = Duration::from_secs(2);
pub(crate) const PUBLIC_DATA_ENCODING: u16 = 0x0001;
pub(crate) const PUBLIC_INDEX_ENCODING: u16 = 0x0002;
const INITIAL_COUNTER: u64 = 0;

/// Uploads a single public scratchpad, gets the store quote, and returns the address and receipt.
async fn upload_and_quote_public_pad(
    network_adapter: &AutonomiNetworkAdapter,
    secret_key: &SecretKey,
    encoding: u16,
    content_bytes: Bytes,
    description: &str, // e.g., "data chunk 0" or "index"
) -> Result<(ScratchpadAddress, Receipt), DataError> {
    let calculated_address = ScratchpadAddress::new(secret_key.public_key());
    let scratchpad = create_public_scratchpad(
        secret_key,
        encoding.into(), // Convert u16 to u64 for the function call
        &content_bytes,
        INITIAL_COUNTER,
    );
    let payment = PaymentOption::Wallet((*network_adapter.wallet()).clone());
    let content_len = content_bytes.len();

    // 1. Put Scratchpad
    let returned_addr = match network_adapter.scratchpad_put(scratchpad, payment).await {
        Ok((_cost, addr)) => {
            if addr != calculated_address {
                warn!(
                    "Address mismatch for public {}: expected {}, got {}",
                    description, calculated_address, addr
                );
            }
            trace!(
                "Put successful for public {} -> {}. Fetching quote...",
                description,
                addr
            );
            addr // Use the returned address
        }
        Err(e) => {
            error!(
                "Put failed for public {} (addr: {}): {}",
                description, calculated_address, e
            );
            return Err(e.into());
        }
    };

    // 2. Get Quote/Receipt
    let receipt = match network_adapter
        .get_store_quotes(
            DataTypes::Scratchpad,
            std::iter::once((XorName::from(returned_addr.xorname()), content_len)),
        )
        .await
    {
        Ok(quotes) => autonomi::client::payment::receipt_from_store_quotes(quotes),
        Err(e) => {
            error!(
                "Failed to get store quote for public {} -> {}: {}",
                description, returned_addr, e
            );
            return Err(e.into());
        }
    };

    Ok((returned_addr, receipt))
}

/// Verifies a public index scratchpad exists and has the correct encoding and counter.
/// Includes retry logic.
async fn verify_public_index_pad(
    network_adapter: &AutonomiNetworkAdapter,
    address: &ScratchpadAddress,
) -> Result<(), DataError> {
    trace!("Starting verification for index pad {}...", address);
    let mut last_error: Option<DataError> = None;

    for attempt in 0..PUBLIC_CONFIRMATION_RETRY_LIMIT {
        trace!(
            "Verification attempt {} for public index scratchpad {}",
            attempt + 1,
            address
        );
        sleep(PUBLIC_CONFIRMATION_RETRY_DELAY).await;

        match network_adapter.get_raw_scratchpad(address).await {
            Ok(fetched_pad) => {
                if fetched_pad.data_encoding() == u64::from(PUBLIC_INDEX_ENCODING) {
                    if fetched_pad.counter() == INITIAL_COUNTER {
                        info!("Public index scratchpad {} verified successfully.", address);
                        return Ok(()); // Verification successful
                    } else {
                        warn!(
                            "Verification attempt {}: Public index {} has unexpected counter {} (expected {}). Retrying...",
                            attempt + 1, address, fetched_pad.counter(), INITIAL_COUNTER
                        );
                        last_error = Some(DataError::InconsistentState(format!(
                            "Public index {} unexpected counter {}",
                            address,
                            fetched_pad.counter()
                        )));
                    }
                } else {
                    warn!(
                        "Verification attempt {}: Public index {} has incorrect encoding {} (expected {}). Retrying...",
                        attempt + 1, address, fetched_pad.data_encoding(), PUBLIC_INDEX_ENCODING
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
                    attempt + 1, address, e_str
                );
                last_error = Some(DataError::PublicScratchpadNotFound(*address));
            }
            Err(e) => {
                warn!(
                    "Verification attempt {}: Unexpected error fetching public index {}: {}. Retrying...",
                    attempt + 1, address, e
                );
                last_error = Some(DataError::Network(e));
            }
        }
    }

    // If loop finishes, verification failed
    let final_error = last_error.unwrap_or_else(|| {
        DataError::InternalError(format!("Verification timed out for index {}", address))
    });
    error!(
        "Failed to verify public index scratchpad {} after {} attempts: {}",
        address, PUBLIC_CONFIRMATION_RETRY_LIMIT, final_error
    );
    Err(final_error)
}

/// Represents the type of pad being uploaded in a public store operation.
pub(crate) enum PublicPadType {
    Data {
        chunk_index: usize,
        chunk_data: Bytes,
    },
    Index {
        index_data: Bytes,
    },
}

/// Input for a single public upload task.
pub(crate) struct PublicUploadTaskInput {
    pub pad_info: PadInfo, // Initial PadInfo with sk_bytes
    pub pad_type: PublicPadType,
}

/// Output from a completed data pad upload task.
pub(crate) struct DataPadUploadOutput {
    pub chunk_index: usize,
    pub address: ScratchpadAddress,
    pub receipt: Receipt,
}

/// Output from a completed and verified index pad upload task.
pub(crate) struct IndexPadUploadOutput {
    pub address: ScratchpadAddress,
    pub receipt: Receipt,
}

/// Represents the result of a single public upload task.
pub(crate) enum PublicUploadTaskResult {
    DataPadComplete(Result<DataPadUploadOutput, (usize, DataError)>),
    IndexPadComplete(Result<IndexPadUploadOutput, DataError>),
}

/// Executes the parallel upload and verification tasks for a public store operation.
///
/// Updates the `IndexManager` incrementally as tasks succeed.
/// Stops processing on the first error or cancellation.
pub(crate) async fn execute_public_upload_tasks(
    index_manager: Arc<DefaultIndexManager>,
    network_adapter: Arc<AutonomiNetworkAdapter>,
    name: String, // The name identifier for the public upload
    tasks_to_run: Vec<PublicUploadTaskInput>,
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<(), DataError> {
    debug!(
        "Executing {} public upload tasks for '{}'...",
        tasks_to_run.len(),
        name
    );
    let mut upload_futures = FuturesUnordered::new();
    let total_tasks = tasks_to_run.len();
    let mut completed_tasks = 0;
    let mut operation_error: Option<DataError> = None;
    let mut callback_cancelled = false;

    for task_input in tasks_to_run {
        let network_adapter_clone = Arc::clone(&network_adapter);
        let index_manager_clone = Arc::clone(&index_manager);
        let callback_arc_clone = Arc::clone(&callback_arc);
        let name_clone = name.clone();

        upload_futures.push(tokio::spawn(async move {
            // --- Task Execution Logic ---
            let initial_pad_info = task_input.pad_info;
            let sk_bytes = initial_pad_info.sk_bytes.clone();
            let calculated_address = initial_pad_info.address;

            // Reconstruct Secret Key (Error handling remains here as it's task setup)
            let secret_key = match <[u8; 32]>::try_from(sk_bytes.as_slice()) {
                Ok(sk_array) => match SecretKey::from_bytes(sk_array) {
                    Ok(sk) => sk,
                    Err(e) => {
                        error!(
                            "Failed to construct SecretKey from bytes array for pad {}: {}",
                            calculated_address, e
                        );
                        return match task_input.pad_type {
                            PublicPadType::Data { chunk_index, .. } => {
                                PublicUploadTaskResult::DataPadComplete(Err((
                                    chunk_index,
                                    DataError::InternalError(format!("Invalid SK bytes: {}", e)),
                                )))
                            }
                            PublicPadType::Index { .. } => {
                                PublicUploadTaskResult::IndexPadComplete(Err(
                                    DataError::InternalError(format!("Invalid SK bytes: {}", e)),
                                ))
                            }
                        };
                    }
                },
                Err(_) => {
                    error!(
                        "Failed to convert Vec<u8> to [u8; 32] for SecretKey for pad {}",
                        calculated_address
                    );
                    return match task_input.pad_type {
                        PublicPadType::Data { chunk_index, .. } => {
                            PublicUploadTaskResult::DataPadComplete(Err((
                                chunk_index,
                                DataError::InternalError("Incorrect SK length".to_string()),
                            )))
                        }
                        PublicPadType::Index { .. } => PublicUploadTaskResult::IndexPadComplete(
                            Err(DataError::InternalError("Incorrect SK length".to_string())),
                        ),
                    };
                }
            };

            // Process based on pad type
            match task_input.pad_type {
                PublicPadType::Data {
                    chunk_index,
                    chunk_data,
                } => {
                    let description = format!("data chunk {}", chunk_index);
                    match upload_and_quote_public_pad(
                        &network_adapter_clone,
                        &secret_key,
                        PUBLIC_DATA_ENCODING,
                        chunk_data,
                        &description,
                    )
                    .await
                    {
                        Ok((returned_addr, receipt)) => {
                            trace!(
                                "Data pad upload complete for chunk {} -> {}",
                                chunk_index,
                                returned_addr
                            );
                            PublicUploadTaskResult::DataPadComplete(Ok(DataPadUploadOutput {
                                chunk_index,
                                address: returned_addr,
                                receipt,
                            }))
                        }
                        Err(e) => PublicUploadTaskResult::DataPadComplete(Err((chunk_index, e))),
                    }
                }
                PublicPadType::Index { index_data } => {
                    let description = "index";
                    // 1. Upload and get quote
                    match upload_and_quote_public_pad(
                        &network_adapter_clone,
                        &secret_key,
                        PUBLIC_INDEX_ENCODING,
                        index_data,
                        description,
                    )
                    .await
                    {
                        Ok((returned_addr, receipt)) => {
                            // 2. Verify the uploaded index pad
                            match verify_public_index_pad(&network_adapter_clone, &returned_addr)
                                .await
                            {
                                Ok(_) => {
                                    trace!(
                                        "Index pad verification successful for {}",
                                        returned_addr
                                    );
                                    PublicUploadTaskResult::IndexPadComplete(Ok(
                                        IndexPadUploadOutput {
                                            address: returned_addr,
                                            receipt,
                                        },
                                    ))
                                }
                                Err(verification_error) => {
                                    PublicUploadTaskResult::IndexPadComplete(Err(
                                        verification_error,
                                    ))
                                }
                            }
                        }
                        Err(upload_error) => {
                            PublicUploadTaskResult::IndexPadComplete(Err(upload_error))
                        }
                    }
                }
            }
            // --- End Task Execution Logic ---
        }));
    }

    debug!(
        "Processing {} public upload futures for '{}'...",
        upload_futures.len(),
        name
    );

    while let Some(join_result) = upload_futures.next().await {
        if callback_cancelled || operation_error.is_some() {
            trace!("Skipping result processing due to previous error or cancellation.");
            continue;
        }

        let task_result = match join_result {
            Ok(res) => res,
            Err(join_err) => {
                error!(
                    "Public upload task panicked or was cancelled for '{}': {}",
                    name, join_err
                );
                operation_error = Some(DataError::InternalError(format!(
                    "Upload task failed unexpectedly: {}",
                    join_err
                )));
                break;
            }
        };

        match task_result {
            PublicUploadTaskResult::DataPadComplete(Ok(output)) => {
                trace!(
                    "Task completed: Data pad chunk {} uploaded to {}",
                    output.chunk_index,
                    output.address
                );
                match index_manager
                    .update_public_data_pad_info(
                        &name,
                        output.chunk_index,
                        PadStatus::Written, // Mark as Written after successful put/quote
                        Some(output.receipt),
                    )
                    .await
                {
                    Ok(_) => {
                        completed_tasks += 1;
                        trace!(
                            "Index updated for data chunk {}. Completed {}/{}",
                            output.chunk_index,
                            completed_tasks,
                            total_tasks
                        );
                        if !invoke_put_callback(
                            &mut *callback_arc.lock().await,
                            PutEvent::ChunkWritten {
                                chunk_index: output.chunk_index,
                            },
                        )
                        .await
                        .map_err(|e| DataError::CallbackError(e.to_string()))?
                        {
                            warn!(
                                "Public store for '{}' cancelled after chunk {} write.",
                                name, output.chunk_index
                            );
                            operation_error = Some(DataError::OperationCancelled);
                            callback_cancelled = true;
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to update index for data chunk {}: {}. Halting.",
                            output.chunk_index, e
                        );
                        operation_error = Some(DataError::Index(e));
                        break;
                    }
                }
            }
            PublicUploadTaskResult::DataPadComplete(Err((chunk_index, e))) => {
                error!(
                    "Task failed: Data pad chunk {} upload error: {}. Halting.",
                    chunk_index, e
                );
                operation_error = Some(e);
                break;
            }
            PublicUploadTaskResult::IndexPadComplete(Ok(output)) => {
                trace!(
                    "Task completed: Index pad uploaded and verified at {}",
                    output.address
                );
                match index_manager
                    .update_public_index_pad_info(
                        &name,
                        PadStatus::Written, // Mark as Written after successful verification
                        Some(output.receipt),
                    )
                    .await
                {
                    Ok(_) => {
                        completed_tasks += 1;
                        trace!(
                            "Index updated for index pad. Completed {}/{}",
                            completed_tasks,
                            total_tasks
                        );
                        // Consider adding a PutEvent::IndexVerified if needed
                    }
                    Err(e) => {
                        error!("Failed to update index for index pad: {}. Halting.", e);
                        operation_error = Some(DataError::Index(e));
                        break;
                    }
                }
            }
            PublicUploadTaskResult::IndexPadComplete(Err(e)) => {
                error!(
                    "Task failed: Index pad upload/verification error: {}. Halting.",
                    e
                );
                operation_error = Some(e);
                break;
            }
        }
    }

    // If loop finished early due to error/cancellation, drain remaining futures
    if operation_error.is_some() || callback_cancelled {
        debug!(
            "Aborting remaining public upload tasks for '{}' due to error or cancellation.",
            name
        );
        while upload_futures.next().await.is_some() {}
    }

    match operation_error {
        Some(err) => Err(err),
        None => {
            if completed_tasks == total_tasks {
                debug!(
                    "All {} public upload tasks completed successfully for '{}'",
                    total_tasks, name
                );
                Ok(())
            } else {
                error!("Public upload for '{}' finished but task count mismatch ({} vs {}). Internal error.", name, completed_tasks, total_tasks);
                Err(DataError::InternalError("Task count mismatch".to_string()))
            }
        }
    }
}
