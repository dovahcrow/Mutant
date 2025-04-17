use crate::data::error::DataError;
use crate::data::ops::common::{DataManagerDependencies, WriteTaskInput};
use crate::events::PutCallback;
use crate::events::{invoke_put_callback, PutEvent};
use crate::index::{structure::PadStatus, KeyInfo, PadInfo};
use crate::pad_lifecycle::PadOrigin;
use autonomi::SecretKey;
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, error, info, trace, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) async fn prepare_pads_for_store(
    deps: &DataManagerDependencies,
    user_key: &str,
    data_size: usize,
    chunks: &[Vec<u8>],
    callback_arc: Arc<Mutex<Option<PutCallback>>>,
) -> Result<(KeyInfo, Vec<WriteTaskInput>), DataError> {
    let _chunk_size = deps.index_manager.get_scratchpad_size().await?;
    let num_chunks = chunks.len();
    let mut tasks_to_run: Vec<WriteTaskInput> = Vec::new();

    let existing_key_info_opt = deps.index_manager.get_key_info(user_key).await?;

    match existing_key_info_opt {
        Some(key_info) => {
            info!(
                "Prepare: Found existing KeyInfo for key '{}'. Attempting resume.",
                user_key
            );
            if key_info.is_complete {
                info!(
                    "Prepare: Key '{}' is already marked as complete. Nothing to do.",
                    user_key
                );
                return Ok((key_info, Vec::new()));
            }

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
                let network_adapter = Arc::clone(&deps.network_adapter);
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
                    let acquired = deps
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
                                if let Err(e) = deps
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
                let network_choice = deps.network_adapter.get_network_choice();
                if let Err(cache_err) = deps
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

                        let pad_exists_known = match pad_info.origin {
                            PadOrigin::FreePool { .. } => true,
                            PadOrigin::Generated => existence_results
                                .get(&pad_info.address)
                                .copied()
                                .unwrap_or(false),
                        };

                        if pad_exists_known {
                            debug!(
                                "Prepare: Task: Write chunk {} to pad {} (Origin: {:?}, Status: {:?})",
                                pad_info.chunk_index, pad_info.address, pad_info.origin, pad_info.status
                            );
                            tasks_to_run.push(WriteTaskInput {
                                pad_info: pad_info.clone(),
                                secret_key,
                                chunk_data: chunk.clone(),
                            });
                        } else {
                            warn!("Prepare: Skipping write task for pad {} which was found not to exist and wasn't replaced.", pad_info.address);
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
            deps.index_manager
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

                deps.index_manager
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
            let acquired_pads_with_origin = deps
                .pad_lifecycle_manager
                .acquire_pads(num_chunks, &mut *callback_arc.lock().await)
                .await?;

            debug!(
                "Prepare: Successfully acquired {} pads.",
                acquired_pads_with_origin.len()
            );

            let mut initial_pads = Vec::with_capacity(num_chunks);
            let mut initial_pad_keys = HashMap::with_capacity(num_chunks);

            for (i, chunk) in chunks.iter().enumerate() {
                if i >= acquired_pads_with_origin.len() {
                    error!(
                        "Prepare: Logic error: Acquired pads count ({}) less than chunk index ({})",
                        acquired_pads_with_origin.len(),
                        i
                    );
                    return Err(DataError::InternalError(
                        "Pad acquisition count mismatch".to_string(),
                    ));
                }
                let (pad_address, secret_key, pad_origin) = acquired_pads_with_origin[i].clone();

                let initial_status = match pad_origin {
                    PadOrigin::Generated => PadStatus::Generated,
                    PadOrigin::FreePool { .. } => PadStatus::Allocated,
                };

                let pad_info = PadInfo {
                    address: pad_address,
                    chunk_index: i,
                    status: initial_status.clone(),
                    origin: pad_origin.clone(),
                    needs_reverification: false,
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

            deps.index_manager
                .insert_key_info(user_key.to_string(), key_info.clone())
                .await?;
            info!(
                "Prepare: Initial KeyInfo for '{}' created with {} pads and persisted.",
                user_key,
                key_info.pads.len()
            );

            let network_choice = deps.network_adapter.get_network_choice();
            if let Err(e) = deps
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
