use super::PadManager;
use crate::error::Error;
use crate::events::{invoke_get_callback, GetCallback, GetEvent};
use crate::utils::retry::retry_operation;
use autonomi::SecretKey;
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::{self, JoinSet};
use tokio::time::{Duration, Instant};

const PROGRESS_UPDATE_THRESHOLD_BYTES: u64 = 1024 * 1024;
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(100);

impl PadManager {
    /// Retrieves the pad information (list of pads and total expected size) for a given key.
    async fn get_pads_and_size(
        &self,
        key: &str,
    ) -> Result<(Vec<(autonomi::ScratchpadAddress, Vec<u8>)>, usize), Error> {
        let mis_guard = self.master_index_storage.lock().await;
        debug!("Retrieve[{}]: Lock acquired to get pad info.", key);
        match mis_guard.index.get(key) {
            Some(key_info) => {
                let pads = key_info.pads.clone();
                let size = key_info.data_size;
                debug!(
                    "Retrieve[{}]: Found {} pads, expected size {}. Releasing lock.",
                    key,
                    pads.len(),
                    size
                );
                Ok((pads, size))
            }
            None => {
                debug!("Retrieve[{}]: Key not found. Releasing lock.", key);
                Err(Error::KeyNotFound(key.to_string()))
            }
        }
    }

    /// Spawns concurrent tasks to fetch individual scratchpads.
    fn spawn_fetch_tasks(
        &self,
        key_owned: &str,
        pads_to_fetch: &[(autonomi::ScratchpadAddress, Vec<u8>)],
        join_set: &mut JoinSet<Result<(usize, Vec<u8>), (usize, Error)>>,
    ) {
        for (index, (pad_address, pad_key_bytes)) in pads_to_fetch.iter().enumerate() {
            let storage_clone = self.storage.clone();
            let address_clone = pad_address.clone();
            let key_bytes_clone = pad_key_bytes.clone();
            let key_for_task = key_owned.to_string();

            debug!(
                "Retrieve[{}]: Spawning task for pad index {} (Addr: {})",
                key_for_task, index, address_clone
            );
            join_set.spawn(async move {
                let secret_key = {
                    let key_array: [u8; 32] = match key_bytes_clone
                        .as_slice()
                        .try_into()
                     {
                         Ok(arr) => arr,
                         Err(_) => {
                             error!(
                                "RetrieveTask[{}]: Invalid key bytes slice length for pad index {}: {}",
                                key_for_task, index, Error::InternalError("Pad key bytes have incorrect length".to_string())
                             );
                             return Err((index, Error::InternalError("Pad key bytes have incorrect length".to_string())));
                         }
                     };
                    match SecretKey::from_bytes(key_array) {
                        Ok(k) => k,
                        Err(e) => {
                            error!(
                                "RetrieveTask[{}]: Failed to create SecretKey from bytes for pad index {}: {}",
                                key_for_task, index, e
                            );
                            return Err((index, Error::InternalError(format!("Failed to create SecretKey: {}", e))));
                        }
                    }
                };

                let fetch_desc = format!("Fetch pad {} - {}", index, address_clone);
                let fetch_result = retry_operation(
                    &fetch_desc,
                    || async {
                        let storage_inner = storage_clone.clone();
                        let addr_inner = address_clone.clone();
                        let key_inner = secret_key.clone();

                        let client = storage_inner.get_client().await?;
                        crate::storage::fetch_scratchpad_internal_static(
                            client,
                            &addr_inner,
                            &key_inner,
                        )
                        .await
                    },
                    |e: &Error| {
                        warn!(
                            "RetrieveTask[{}]: Retrying fetch for pad #{} due to error: {}",
                            key_for_task, index, e
                        );
                        true
                    },
                )
                .await;
                let fetched_data = fetch_result.map_err(|e| (index, e))?;

                debug!(
                    "RetrieveTask[{}]: Fetched pad index {} ({} bytes)",
                    key_for_task,
                    index,
                    fetched_data.len()
                );
                Ok((index, fetched_data))
            });
        }
    }

    /// Collects results from fetch tasks, updates progress, and handles errors.
    async fn collect_fetch_results(
        key_owned: &str,
        join_set: &mut JoinSet<Result<(usize, Vec<u8>), (usize, Error)>>,
        shared_callback: Arc<Mutex<&mut Option<GetCallback>>>,
        total_bytes_fetched: Arc<Mutex<u64>>,
        expected_data_size: usize,
    ) -> Result<std::collections::HashMap<usize, Vec<u8>>, Error> {
        let mut fetched_data_map: std::collections::HashMap<usize, Vec<u8>> =
            std::collections::HashMap::with_capacity(join_set.len());
        let mut first_error: Option<Error> = None;
        let mut local_bytes_batch: u64 = 0;
        let mut last_update_time = Instant::now();

        while let Some(join_res) = join_set.join_next().await {
            match join_res {
                Ok(task_result) => match task_result {
                    Ok((index, data)) => {
                        let bytes_downloaded = data.len() as u64;
                        debug!(
                            "Retrieve[{}]: Task {} completed, got {} bytes.",
                            key_owned, index, bytes_downloaded
                        );
                        if bytes_downloaded > 0 {
                            local_bytes_batch += bytes_downloaded;
                            let current_time = Instant::now();
                            let time_since_last_update =
                                current_time.duration_since(last_update_time);

                            if local_bytes_batch >= PROGRESS_UPDATE_THRESHOLD_BYTES
                                || time_since_last_update >= PROGRESS_UPDATE_INTERVAL
                            {
                                if let Err(e) = Self::update_progress(
                                    key_owned,
                                    &shared_callback,
                                    &total_bytes_fetched,
                                    &mut local_bytes_batch,
                                    expected_data_size,
                                    "progress update",
                                )
                                .await
                                {
                                    if first_error.is_none() {
                                        first_error = Some(e);
                                    }
                                }
                                last_update_time = current_time;
                            }
                        }
                        fetched_data_map.insert(index, data);
                    }
                    Err((index, e)) => {
                        error!(
                            "Retrieve[{}]: Failed task for pad index {}: {}",
                            key_owned, index, e
                        );
                        if first_error.is_none() {
                            first_error = Some(e);
                            join_set.abort_all();
                        }
                    }
                },
                Err(join_error) => {
                    error!("Retrieve[{}]: JoinError: {}", key_owned, join_error);
                    if first_error.is_none() {
                        first_error = Some(Error::from_join_error_msg(
                            &join_error,
                            "Retrieve sub-task join error".to_string(),
                        ));
                        join_set.abort_all();
                    }
                }
            }
            if first_error.is_some() {
                while let Some(_) = join_set.join_next().await {}
                break;
            }
        }

        if local_bytes_batch > 0 && first_error.is_none() {
            debug!(
                "Retrieve[{}]: Performing final progress update for remaining {} bytes.",
                key_owned, local_bytes_batch
            );
            if let Err(e) = Self::update_progress(
                key_owned,
                &shared_callback,
                &total_bytes_fetched,
                &mut local_bytes_batch,
                expected_data_size,
                "final progress update",
            )
            .await
            {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }

        if let Some(e) = first_error {
            return Err(e);
        }

        Ok(fetched_data_map)
    }

    /// Helper to update download progress and invoke the callback.
    async fn update_progress(
        key_owned: &str,
        shared_callback: &Arc<Mutex<&mut Option<GetCallback>>>,
        total_bytes_fetched: &Arc<Mutex<u64>>,
        local_bytes_batch: &mut u64,
        expected_data_size: usize,
        context: &str,
    ) -> Result<(), Error> {
        let current_total_fetched = {
            let mut total_guard = total_bytes_fetched.lock().await;
            *total_guard += *local_bytes_batch;
            *total_guard
        };
        debug!(
            "Retrieve[{}]: Updating {}. Batch size: {}. New total: {} / {}",
            key_owned, context, *local_bytes_batch, current_total_fetched, expected_data_size
        );

        *local_bytes_batch = 0;

        let mut guard = shared_callback.lock().await;
        if guard.is_some() {
            invoke_get_callback(
                &mut **guard,
                GetEvent::DownloadProgress {
                    bytes_read: current_total_fetched,
                    total_bytes: expected_data_size as u64,
                },
            )
            .await
            .map_err(|e| {
                error!(
                    "Retrieve[{}]: Callback error during {}: {}",
                    key_owned, context, e
                );
                e
            })?;
        }
        Ok(())
    }

    /// Orders the fetched data chunks and combines them into a single Vec<u8>.
    async fn order_and_combine_data(
        key_owned: &str,
        fetched_data_map: std::collections::HashMap<usize, Vec<u8>>,
        num_pads: usize,
        expected_data_size: usize,
    ) -> Result<Vec<u8>, Error> {
        if fetched_data_map.len() != num_pads {
            error!(
                "Retrieve[{}]: Mismatch in fetched pads. Expected: {}, Got: {}",
                key_owned,
                num_pads,
                fetched_data_map.len()
            );
            return Err(Error::InternalError(
                "Failed to fetch all required pads (tasks might have been aborted or missing)"
                    .to_string(),
            ));
        }

        let mut ordered_data: Vec<Option<Vec<u8>>> = vec![None; num_pads];
        for (index, data) in fetched_data_map.into_iter() {
            if index < num_pads {
                ordered_data[index] = Some(data);
            } else {
                error!(
                    "Retrieve[{}]: Invalid index {} found in fetched data map (max expected {}).",
                    key_owned,
                    index,
                    num_pads - 1
                );
                return Err(Error::InternalError(
                    "Invalid index encountered during data preparation for combine".to_string(),
                ));
            }
        }

        let final_ordered_data = ordered_data
            .into_iter()
            .enumerate()
            .map(|(i, opt_data)| {
                opt_data.ok_or_else(|| {
                    error!(
                        "Retrieve[{}]: Missing data for pad index {} during final combine preparation.",
                        key_owned, i
                    );
                    Error::InternalError("Pad data map missing index during combine prep".to_string())
                })
            })
            .collect::<Result<Vec<Vec<u8>>, Error>>()?;

        debug!(
            "Retrieve[{}]: Combining data from {} pads in blocking task...",
            key_owned,
            final_ordered_data.len()
        );

        let combine_result = task::spawn_blocking(move || {
            let mut combined_data = Vec::with_capacity(expected_data_size);
            for data_chunk in final_ordered_data {
                combined_data.extend_from_slice(&data_chunk);
            }
            combined_data
        })
        .await;

        let mut combined_data = match combine_result {
            Ok(data) => data,
            Err(join_error) => {
                error!(
                    "Retrieve[{}]: JoinError during blocking combine: {}",
                    key_owned, join_error
                );
                return Err(Error::from_join_error_msg(
                    &join_error,
                    "Combine sub-task join error".to_string(),
                ));
            }
        };

        if combined_data.len() != expected_data_size {
            warn!(
                "Retrieve[{}]: Combined data size ({}) differs from expected ({}). Truncating.",
                key_owned,
                combined_data.len(),
                expected_data_size
            );
            combined_data.truncate(expected_data_size);
        } else {
            debug!(
                "Retrieve[{}]: Combined data size matches expected size: {}",
                key_owned, expected_data_size
            );
        }

        Ok(combined_data)
    }

    /// Retrieves data associated with a key by fetching and combining its constituent scratchpads.
    ///
    /// This function orchestrates the retrieval process:
    /// 1. Looks up the key in the master index to get pad addresses and expected size.
    /// 2. Spawns concurrent tasks to fetch data from each pad's storage location.
    /// 3. Collects results, manages progress updates via callbacks, and handles errors.
    /// 4. Orders the fetched data chunks correctly.
    /// 5. Combines the ordered chunks into the final data blob.
    /// 6. Invokes callbacks for start, progress, and completion events.
    pub async fn retrieve_data(
        &self,
        key: &str,
        callback: &mut Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        info!("PadManager::Retrieve[{}]: Starting retrieval...", key);
        let key_owned = key.to_string();

        let (pads_to_fetch, expected_data_size) = self.get_pads_and_size(key).await?;

        if pads_to_fetch.is_empty() {
            debug!(
                "Retrieve[{}]: No pads associated with key, returning empty data.",
                key
            );
            if expected_data_size > 0 {
                warn!(
                    "Retrieve[{}]: Key found but has no pads, yet expected size is {}. Returning empty.",
                    key, expected_data_size
                );
            }
            invoke_get_callback(callback, GetEvent::StartingDownload { total_bytes: 0 }).await?;
            invoke_get_callback(callback, GetEvent::DownloadFinished).await?;
            return Ok(Vec::new());
        }

        let mut join_set = JoinSet::new();
        let total_bytes_expected = expected_data_size as u64;

        let shared_callback = Arc::new(Mutex::new(&mut *callback));
        let total_bytes_fetched = Arc::new(Mutex::new(0u64));

        {
            let mut cb_guard = shared_callback.lock().await;
            invoke_get_callback(
                &mut *cb_guard,
                GetEvent::StartingDownload {
                    total_bytes: total_bytes_expected,
                },
            )
            .await?;
        }

        self.spawn_fetch_tasks(&key_owned, &pads_to_fetch, &mut join_set);

        let fetched_data_map_result = Self::collect_fetch_results(
            &key_owned,
            &mut join_set,
            Arc::clone(&shared_callback),
            Arc::clone(&total_bytes_fetched),
            expected_data_size,
        )
        .await;

        let owned_callback_opt: Option<&mut Option<GetCallback>> = Arc::try_unwrap(shared_callback)
            .ok()
            .map(|mutex| mutex.into_inner());

        let fetched_data_map = match fetched_data_map_result {
            Ok(map) => map,
            Err(e) => {
                return Err(e);
            }
        };

        let combined_data = Self::order_and_combine_data(
            &key_owned,
            fetched_data_map,
            pads_to_fetch.len(),
            expected_data_size,
        )
        .await?;

        debug!(
            "Retrieve[{}]: Invoking DownloadFinished callback...",
            key_owned
        );
        if let Some(original_callback_ref) = owned_callback_opt {
            invoke_get_callback(original_callback_ref, GetEvent::DownloadFinished).await?;
        } else {
            error!("Retrieve[{}]: Failed to regain exclusive ownership of the callback reference after successful download.", key_owned);
            return Err(Error::InternalError(
                "Callback ownership lost after success".to_string(),
            ));
        }

        debug!(
            "Retrieve[{}]: Successfully retrieved and combined {} bytes.",
            key_owned,
            combined_data.len()
        );
        Ok(combined_data)
    }
}
