use crate::data::error::DataError;
use crate::data::ops::common::WriteTaskInput;
use crate::index::{structure::PadStatus, KeyInfo, PadInfo};
use crate::internal_events::PutCallback;
use crate::internal_events::{invoke_put_callback, PutEvent};
use crate::pad_lifecycle::PadOrigin;
use autonomi::SecretKey;
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
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

            let mut checks_needed = Vec::new();
            for pad_info in key_info.pads.iter() {
                if pad_info.status == PadStatus::Generated
                    && pad_info.origin == PadOrigin::Generated
                {
                    checks_needed.push(pad_info.address);
                }
            }

            let mut existence_results = HashMap::new();
            let mut pads_needing_replacement = Vec::new();

            if !checks_needed.is_empty() {
                debug!(
                    "Prepare: Performing concurrent existence checks for {} generated pads...",
                    checks_needed.len()
                );
                let mut check_futures = FuturesUnordered::new();
                let network_adapter = Arc::clone(&data_manager.network_adapter);
                for address in checks_needed {
                    let net_clone = Arc::clone(&network_adapter);
                    check_futures.push(async move {
                        let result = net_clone.check_existence(&address).await;
                        (address, result)
                    });
                }

                while let Some((address, result)) = check_futures.next().await {
                    match result {
                        Ok(exists) => {
                            existence_results.insert(address, exists);
                        }
                        Err(e) => {
                            let error_string = e.to_string();
                            if error_string.contains("NotEnoughCopies") {
                                warn!(
                                    "Prepare: Network existence check for pad {} failed with NotEnoughCopies. Marking for replacement.",
                                    address
                                );
                                pads_needing_replacement.push(address);
                                existence_results.insert(address, false);
                            } else {
                                error!(
                                    "Prepare: Network error checking existence for pad {} during resume preparation: {}. Aborting.",
                                    address, e
                                );
                                return Err(DataError::InternalError(format!(
                                    "Network existence check failed during resume preparation: {}",
                                    e
                                )));
                            }
                        }
                    }
                }
                debug!("Prepare: Concurrent existence checks complete.");
            }

            let mut key_info_mut = key_info;
            if !pads_needing_replacement.is_empty() {
                info!(
                    "Prepare: Replacing {} pads that failed existence check...",
                    pads_needing_replacement.len()
                );
                key_info_mut.is_complete = false;

                for old_address in pads_needing_replacement {
                    debug!("Prepare: Acquiring replacement for pad {}", old_address);
                    let acquired = data_manager
                        .pad_lifecycle_manager
                        .acquire_pads(1, &mut *callback_arc.lock().await)
                        .await?;

                    if acquired.len() == 1 {
                        let (new_address, new_secret_key, new_origin) =
                            acquired.into_iter().next().unwrap();
                        info!(
                            "Prepare: Acquired replacement pad {} for original pad {}",
                            new_address, old_address
                        );

                        if let Some(pad_info_mut) = key_info_mut
                            .pads
                            .iter_mut()
                            .find(|p| p.address == old_address)
                        {
                            if let Some(old_key_bytes) =
                                key_info_mut.pad_keys.get(&old_address).cloned()
                            {
                                trace!(
                                    "Prepare: Adding abandoned pad {} to pending verification list",
                                    old_address
                                );
                                if let Err(e) = data_manager
                                    .index_manager
                                    .add_pending_pads(vec![(old_address, old_key_bytes)])
                                    .await
                                {
                                    warn!("Prepare: Failed to add abandoned pad {} to pending list: {}. Proceeding anyway.", old_address, e);
                                }
                            } else {
                                warn!("Prepare: Could not find key for abandoned pad {} to add to pending list.", old_address);
                            }

                            pad_info_mut.address = new_address;
                            pad_info_mut.origin = new_origin;
                            pad_info_mut.needs_reverification = false;
                            pad_info_mut.status = PadStatus::Generated;

                            key_info_mut.pad_keys.remove(&old_address);
                            key_info_mut
                                .pad_keys
                                .insert(new_address, new_secret_key.to_bytes().to_vec());
                        } else {
                            error!(
                                "Prepare: Logic error: Could not find PadInfo for address {} to replace.",
                                old_address
                            );
                            return Err(DataError::InternalError(format!(
                                "Failed to find pad {} for replacement",
                                old_address
                            )));
                        }
                    } else {
                        error!(
                            "Prepare: Pad acquisition returned {} pads when acquiring replacement.",
                            acquired.len()
                        );
                        return Err(DataError::InternalError(
                            "Unexpected number of pads returned during replacement".to_string(),
                        ));
                    }
                }
                let network_choice = data_manager.network_adapter.get_network_choice();
                if let Err(cache_err) = data_manager
                    .pad_lifecycle_manager
                    .save_index_cache(network_choice)
                    .await
                {
                    warn!(
                        "Prepare: Failed to save index cache after replacing pads: {}. Proceeding anyway.",
                        cache_err
                    );
                } else {
                    debug!("Prepare: Index cache saved after replacing pads.");
                }
            }

            let final_prepared_key_info = &mut key_info_mut;

            debug!("Prepare: Identifying resume tasks based on pad status...");
            for pad_info in final_prepared_key_info.pads.iter() {
                if pad_info.status == PadStatus::Generated
                    || pad_info.status == PadStatus::Allocated
                {
                    if let Some(key_bytes) = final_prepared_key_info.pad_keys.get(&pad_info.address)
                    {
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

                        // Determine if the pad is known to exist based on origin or checks.
                        let should_attempt_write = match pad_info.origin {
                            // Pads from the free pool *should* exist.
                            PadOrigin::FreePool { .. } => true,
                            // For generated pads, trust existence check result, BUT...
                            PadOrigin::Generated => {
                                let exists = existence_results
                                    .get(&pad_info.address)
                                    .copied()
                                    .unwrap_or(false);
                                if !exists {
                                    // ... if check says it *doesn't* exist, log it, but still attempt write.
                                    // This handles potential check_existence inconsistencies seen in devnet.
                                    // The put_raw function has a workaround for "already exists" errors on create.
                                    warn!(
                                        "Prepare: Network check indicated pad {} (Generated) does not exist, but attempting write anyway due to potential inconsistency.",
                                        pad_info.address
                                    );
                                    true // Attempt write despite negative existence check
                                } else {
                                    true // Exists, so definitely attempt write
                                }
                            }
                        };

                        if should_attempt_write {
                            debug!(
                                "Prepare: Task: Write chunk {} to pad {} (Origin: {:?}, Status: {:?})",
                                pad_info.chunk_index, pad_info.address, pad_info.origin, pad_info.status
                            );
                            tasks_to_run.push(WriteTaskInput {
                                pad_info: pad_info.clone(),
                                secret_key: secret_key.clone(),
                                chunk_data: chunk.clone(),
                            });
                        } else {
                            // This branch should ideally not be reached with the new logic above for Generated pads,
                            // but kept for safety / potential future scenarios.
                            warn!("Prepare: Skipping write task for pad {} (Origin: {:?}, Status: {:?}) based on existence check.", pad_info.address, pad_info.origin, pad_info.status);
                        }
                    } else {
                        warn!(
                            "Prepare: Missing secret key for pad {} in KeyInfo during resume preparation. Skipping.",
                            pad_info.address
                        );
                        return Err(DataError::InternalError(format!(
                            "Missing key for pad {} during resume preparation",
                            pad_info.address
                        )));
                    }
                }
            }
            info!("Prepare: Prepared {} tasks for resume.", tasks_to_run.len());

            final_prepared_key_info.modified = Utc::now();
            data_manager
                .index_manager
                .insert_key_info(user_key.to_string(), final_prepared_key_info.clone())
                .await?;

            Ok((final_prepared_key_info.clone(), tasks_to_run))
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
                    pad_keys: HashMap::new(),
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
            let mut initial_pad_keys = HashMap::with_capacity(num_chunks);

            for (i, chunk) in chunks.iter().enumerate() {
                let (pad_address, secret_key, pad_origin) = acquired_pads[i].clone();

                // Combine matches on pad_origin
                let (initial_status, initial_counter) = match pad_origin {
                    PadOrigin::Generated => (PadStatus::Generated, 0),
                    PadOrigin::FreePool { initial_counter } => {
                        (PadStatus::Allocated, initial_counter)
                    }
                };

                let pad_info = PadInfo {
                    chunk_index: i,
                    status: initial_status.clone(), // Clone needed due to use in debug! after move
                    origin: pad_origin,
                    needs_reverification: false,
                    address: pad_address,
                    last_known_counter: initial_counter,
                };

                initial_pads.push(pad_info.clone());
                initial_pad_keys.insert(pad_address, secret_key.to_bytes().to_vec());

                debug!(
                    "Prepare: Task: Write chunk {} to new pad {} (Origin: {:?}, Status: {:?})",
                    i, pad_address, pad_origin, initial_status
                );
                tasks_to_run.push(WriteTaskInput {
                    pad_info,
                    secret_key,
                    chunk_data: chunk.clone(),
                });
            }

            let key_info = KeyInfo {
                pads: initial_pads,
                pad_keys: initial_pad_keys,
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
