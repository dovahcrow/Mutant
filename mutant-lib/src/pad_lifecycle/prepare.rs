use crate::data::error::DataError;
use crate::data::ops::common::WriteTaskInput;
use crate::index::{structure::PadStatus, KeyInfo, PadInfo};
use crate::internal_events::PutCallback;
use crate::internal_events::{invoke_put_callback, PutEvent};
use crate::pad_lifecycle::PadOrigin;
use autonomi::SecretKey;
use chrono::Utc;
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Prepares the necessary pad information and write tasks for a store operation.
///
/// Handles both initial uploads and resuming incomplete uploads.
/// - For new uploads: Acquires the required number of pads (from free pool or newly generated),
///   creates the initial `KeyInfo`, and generates `WriteTaskInput` for all chunks.
/// - For resume: Loads existing `KeyInfo`, checks for consistency (data size, chunk count),
///   performs network existence checks for pads that were originally generated, potentially
///   replaces pads that don't exist or failed checks, and generates `WriteTaskInput` only
///   for pads that are not yet `Confirmed` or `Written`.
///
/// Updates the `KeyInfo` in the `IndexManager` and saves the index cache.
///
/// # Arguments
///
/// * `data_manager` - A reference to the `DefaultDataManager`.
/// * `user_key` - The key associated with the data being stored.
/// * `data_size` - The total size of the data being stored.
/// * `chunks` - A slice of byte vectors, each representing a chunk of the data.
/// * `callback_arc` - An `Arc<Mutex<Option<PutCallback>>>` for reporting progress events.
///
/// # Errors
///
/// Returns `DataError` if:
/// - Getting scratchpad size fails (`DataError::Index`).
/// - Getting existing key info fails (`DataError::Index`).
/// - Resuming and data size/chunk count mismatch (`DataError::InconsistentState`).
/// - Callback cancels operation (`DataError::OperationCancelled`).
/// - Network existence check fails (`DataError::InternalError`, wrapped network error).
/// - Acquiring replacement pads fails (`DataError::PadLifecycle`).
/// - Updating index fails (`DataError::Index`).
/// - Saving cache fails (warning logged, but returns Ok).
/// - Internal logic errors (missing keys, index out of bounds) (`DataError::InternalError`).
/// - Callback invocation fails (`DataError::InternalError`).
///
/// # Returns
///
/// A tuple containing:
/// * The prepared `KeyInfo` (either newly created or the updated existing one).
/// * A `Vec<WriteTaskInput>` containing the tasks needed to write/confirm data chunks.
///   This might be empty if the key was already complete or if storing empty data.
pub(crate) async fn prepare_pads_for_store(
    data_manager: &crate::data::manager::DefaultDataManager,
    user_key: &str,
    data_size: usize,
    chunks: &[Vec<u8>],
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<(KeyInfo, Vec<WriteTaskInput>), DataError> {
    let _chunk_size = data_manager.index_manager.get_scratchpad_size().await?;
    let num_chunks = chunks.len();
    let mut tasks_to_run: Vec<WriteTaskInput> = Vec::new();

    let existing_key_info_opt = data_manager.index_manager.get_key_info(user_key).await?;

    match existing_key_info_opt {
        Some(key_info) => {
            info!(
                "Prepare: Found existing KeyInfo for key '{}'. Checking for resume/overwrite...",
                user_key
            );

            if key_info.is_complete {
                error!(
                    "Prepare: Attempting to store to key '{}' which already exists and is complete.",
                    user_key
                );
                return Err(DataError::KeyAlreadyExists(user_key.to_string()));
            }

            info!("Prepare: Resuming upload for key '{}'.", user_key);

            if key_info.data_size != data_size {
                error!(
                    "Prepare: Data size mismatch for key '{}'. Expected {}, got {}. Cannot resume.",
                    user_key, key_info.data_size, data_size
                );
                return Err(DataError::InconsistentState(format!(
                    "Data size mismatch for key '{}'",
                    user_key
                )));
            }
            if key_info.pads.len() != num_chunks {
                error!(
                    "Prepare: Chunk count mismatch for key '{}'. Index has {}, data has {}. Cannot resume.",
                    user_key,
                    key_info.pads.len(),
                    num_chunks
                );
                return Err(DataError::InconsistentState(format!(
                    "Chunk count mismatch for key '{}'",
                    user_key
                )));
            }

            let mut initial_written_count = 0;
            let mut initial_confirmed_count = 0;
            for pad in &key_info.pads {
                match pad.status {
                    PadStatus::Written => initial_written_count += 1,
                    PadStatus::Confirmed => {
                        initial_written_count += 1;
                        initial_confirmed_count += 1;
                    }
                    PadStatus::Generated | PadStatus::Allocated => {}
                }
            }

            if !invoke_put_callback(
                &mut *callback_arc.lock().await,
                PutEvent::Starting {
                    total_chunks: num_chunks,
                    initial_written_count,
                    initial_confirmed_count,
                },
            )
            .await
            .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
            {
                warn!(
                    "Prepare: Put operation cancelled by callback during Starting event (resume)."
                );
                return Err(DataError::OperationCancelled);
            }

            let mut key_info_mut = key_info;

            debug!("Prepare: Identifying resume tasks based on pad status...");
            for pad_info in key_info_mut.pads.iter_mut() {
                if pad_info.status == PadStatus::Generated
                    || pad_info.status == PadStatus::Allocated
                {
                    // Ensure sk_bytes exist before proceeding
                    if pad_info.sk_bytes.is_empty() {
                        error!(
                            "Pad {} for existing key '{}' is missing its secret key bytes. Cannot reuse.",
                            pad_info.address, user_key
                        );
                        return Err(DataError::InternalError(format!(
                            "Missing key for pad {} during reuse prep",
                            pad_info.address
                        )));
                    }
                    // sk_bytes are available, proceed with reuse logic
                    let key_bytes = &pad_info.sk_bytes; // Use directly

                    let secret_key =
                        SecretKey::from_bytes(key_bytes.clone().try_into().map_err(|_| {
                            DataError::InternalError("Invalid key size in index".to_string())
                        })?)?;
                    let chunk = chunks.get(pad_info.chunk_index).ok_or_else(|| {
                        DataError::InternalError(format!(
                            "Chunk index {} out of bounds during resume preparation",
                            pad_info.chunk_index
                        ))
                    })?;

                    debug!(
                        "Prepare: Task: Write chunk {} to pad {} (Origin: {:?}, Status: {:?})",
                        pad_info.chunk_index, pad_info.address, pad_info.origin, pad_info.status
                    );
                    tasks_to_run.push(WriteTaskInput {
                        pad_info: pad_info.clone(),
                        secret_key: secret_key.clone(),
                        chunk_data: chunk.clone(),
                    });

                    pad_info.status = PadStatus::Generated;
                    pad_info.needs_reverification = true;
                }
            }
            info!("Prepare: Prepared {} tasks for resume.", tasks_to_run.len());

            key_info_mut.modified = Utc::now();
            data_manager
                .index_manager
                .insert_key_info(user_key.to_string(), key_info_mut.clone())
                .await?;

            Ok((key_info_mut, tasks_to_run))
        }
        None => {
            info!(
                "Prepare: No existing KeyInfo found for key '{}'. Starting new upload.",
                user_key
            );

            if num_chunks == 0 {
                debug!("Prepare: Storing empty data for key '{}'", user_key);
                let key_info = KeyInfo {
                    pads: Vec::new(),
                    data_size,
                    modified: Utc::now(),
                    is_complete: true,
                };

                data_manager
                    .index_manager
                    .insert_key_info(user_key.to_string(), key_info.clone())
                    .await?;
                info!("Prepare: Empty key '{}' created and persisted.", user_key);

                if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
                    .await
                    .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
                {
                    return Err(DataError::OperationCancelled);
                }

                return Ok((key_info, Vec::new()));
            }

            if !invoke_put_callback(
                &mut *callback_arc.lock().await,
                PutEvent::Starting {
                    total_chunks: num_chunks,
                    initial_written_count: 0,
                    initial_confirmed_count: 0,
                },
            )
            .await
            .map_err(|e| DataError::InternalError(format!("Callback failed: {}", e)))?
            {
                warn!("Prepare: Put operation cancelled by callback during Starting event.");
                return Err(DataError::OperationCancelled);
            }

            debug!("Prepare: Acquiring {} pads...", num_chunks);
            let acquired_pads_result = data_manager
                .pad_lifecycle_manager
                .acquire_pads(num_chunks, &mut *callback_arc.lock().await)
                .await;

            let acquired_pads = match acquired_pads_result {
                Ok(pads) => {
                    if pads.len() == num_chunks {
                        pads
                    } else {
                        error!(
                            "Prepare: Acquired {} pads, but expected {}. Mismatch!",
                            pads.len(),
                            num_chunks
                        );
                        return Err(DataError::InternalError(
                            "Pad acquisition count mismatch".to_string(),
                        ));
                    }
                }
                Err(e) => {
                    error!("Prepare: Failed to acquire pads: {}", e);
                    return Err(DataError::PadLifecycle(e));
                }
            };

            debug!(
                "Prepare: Successfully acquired {} pads.",
                acquired_pads.len()
            );

            let mut initial_pads = Vec::with_capacity(num_chunks);

            for (i, chunk) in chunks.iter().enumerate() {
                let (pad_address, secret_key, pad_origin) = acquired_pads[i].clone();

                // Combine matches on pad_origin
                let (initial_status, initial_counter) = match pad_origin {
                    PadOrigin::Generated => (PadStatus::Generated, 0),
                    PadOrigin::FreePool { initial_counter } => {
                        (PadStatus::Allocated, initial_counter)
                    }
                };

                let new_pad_info = PadInfo {
                    address: pad_address,
                    chunk_index: i, // Use the loop index directly
                    status: PadStatus::Generated,
                    origin: pad_origin,
                    needs_reverification: false,
                    last_known_counter: 0, // New pads start at 0
                    sk_bytes: secret_key.to_bytes().to_vec(),
                    receipt: None, // Initialize receipt as None
                };

                initial_pads.push(new_pad_info.clone());

                debug!(
                    "Prepare: Task: Write chunk {} to new pad {} (Origin: {:?}, Status: {:?})",
                    i, pad_address, pad_origin, initial_status
                );
                tasks_to_run.push(WriteTaskInput {
                    pad_info: new_pad_info,
                    secret_key,
                    chunk_data: chunk.clone(),
                });
            }

            let key_info = KeyInfo {
                pads: initial_pads,
                data_size,
                modified: Utc::now(),
                is_complete: false,
            };

            data_manager
                .index_manager
                .insert_key_info(user_key.to_string(), key_info.clone())
                .await?;
            info!(
                "Prepare: Initial KeyInfo for '{}' created with {} pads and persisted.",
                user_key,
                key_info.pads.len()
            );

            let network_choice = data_manager.network_adapter.get_network_choice();
            if let Err(e) = data_manager
                .pad_lifecycle_manager
                .save_index_cache(network_choice)
                .await
            {
                warn!(
                    "Prepare: Failed to save initial index cache for key '{}': {}. Proceeding anyway.",
                    user_key, e
                );
            } else {
                debug!(
                    "Prepare: Successfully saved initial index cache for key '{}'.",
                    user_key
                );
            }

            Ok((key_info, tasks_to_run))
        }
    }
}
