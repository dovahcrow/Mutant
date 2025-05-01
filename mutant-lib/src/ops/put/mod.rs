use crate::error::Error;
use crate::index::error::IndexError;
use crate::index::{PadInfo, PadStatus};
use crate::internal_events::invoke_put_callback;
use crate::network::client::{ClientManager, Config};
use crate::network::{Network, NetworkError};
use crate::ops::worker::{AsyncTask, PoolError, WorkerPool};
use crate::ops::{BATCH_SIZE, MAX_CONFIRMATION_DURATION, WORKER_COUNT};
use async_channel::bounded;
use async_trait::async_trait;
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use log::{debug, error, info, warn};
use mutant_protocol::{PutCallback, PutEvent, StorageMode};
use std::{
    ops::Range,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::Instant;

use super::{
    DATA_ENCODING_PRIVATE_DATA, DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX,
    PAD_RECYCLING_RETRIES,
};

#[derive(Clone)]
struct Context {
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: Arc<String>,
    data: Arc<Vec<u8>>,
    chunk_ranges: Arc<Vec<Range<usize>>>,
    public: bool,
    index_pad_data: Option<Arc<Vec<u8>>>,
}

pub(super) async fn put(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    content: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    if index.read().await.contains_key(key_name) {
        if index
            .read()
            .await
            .verify_checksum(key_name, &content, mode.clone())
        {
            info!("Resume for {}", key_name);
            resume(
                index,
                network,
                key_name,
                content,
                mode,
                public,
                no_verify,
                put_callback,
            )
            .await
        } else {
            info!("Update for {}", key_name);
            index.write().await.remove_key(key_name).unwrap();
            first_store(
                index,
                network,
                key_name,
                content,
                mode,
                public,
                no_verify,
                put_callback,
            )
            .await
        }
    } else {
        info!("First store for {}", key_name);
        first_store(
            index,
            network,
            key_name,
            content,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await
    }
}

async fn resume(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let pads = index.read().await.get_pads(name);

    if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
        index.write().await.remove_key(name).unwrap();
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await;
    }

    let index_pad_data = if public && pads.len() > 1 {
        let data_pads: Vec<_> = pads.iter().skip(1).cloned().collect();
        Some(Arc::new(serde_cbor::to_vec(&data_pads).map_err(|e| {
            Error::Index(IndexError::SerializationError(e.to_string()))
        })?))
    } else {
        None
    };

    let chunk_ranges = index
        .read()
        .await
        .chunk_data(&data_bytes, mode.clone(), public);

    if pads.len() != chunk_ranges.len() {
        warn!(
            "Resuming key '{}' with data size mismatch. Index has {} pads, current data requires {}. Forcing rewrite.",
            name,
            pads.len(),
            chunk_ranges.len()
        );
        index.write().await.remove_key(name)?;
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await;
    }

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        data: data_bytes.clone(),
        chunk_ranges: Arc::new(chunk_ranges),
        public,
        index_pad_data,
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback).await?;

    Ok(pads[0].address)
}

async fn first_store(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let (pads, chunk_ranges) = index
        .write()
        .await
        .create_key(name, &data_bytes, mode, public)?;

    let index_pad_data = if public && pads.len() > 1 {
        let data_pads: Vec<_> = pads.iter().skip(1).cloned().collect();
        Some(Arc::new(serde_cbor::to_vec(&data_pads).map_err(|e| {
            Error::Index(IndexError::SerializationError(e.to_string()))
        })?))
    } else {
        None
    };

    info!("Created key {} with {} pads", name, pads.len());

    let address = pads[0].address;

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        chunk_ranges: Arc::new(chunk_ranges),
        data: data_bytes.clone(),
        public,
        index_pad_data,
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback).await?;

    Ok(address)
}

#[derive(Clone)]
struct PutTaskContext {
    base_context: Context,
    no_verify: Arc<bool>,
    put_callback: Option<PutCallback>,
    total_pads: usize,
    completion_notifier: Arc<Notify>,
    confirmed_counter: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct PutTaskProcessor;

#[async_trait]
impl AsyncTask<PadInfo, PutTaskContext, Object<ClientManager>, (), Error> for PutTaskProcessor {
    type ItemId = usize;

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<PutTaskContext>,
        client: &Object<ClientManager>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, ()), (Error, PadInfo)> {
        let mut pad = pad;
        let current_pad_address = pad.address;
        let initial_status = pad.status;
        let mut put_succeeded = false;
        let mut pad_after_put = pad.clone();

        let should_put = match initial_status {
            PadStatus::Generated | PadStatus::Free => true,
            PadStatus::Written => false,
            PadStatus::Confirmed => {
                return Ok((pad.chunk_index, ()));
            }
        };

        if should_put {
            let is_index_pad =
                context.base_context.public && pad.chunk_index == 0 && context.total_pads > 1;
            let chunk_data_slice = match (is_index_pad, &context.base_context.index_pad_data) {
                (true, Some(index_data)) => &index_data[..],
                (true, None) => {
                    error!(
                        "Worker {} missing index pad data for pad {}",
                        worker_id, current_pad_address
                    );
                    return Err((Error::Internal("Missing index pad data".to_string()), pad));
                }
                (false, _) => {
                    let chunk_index = pad.chunk_index;
                    let range = context.base_context.chunk_ranges[chunk_index].clone();
                    if range.end > context.base_context.data.len() {
                        error!(
                            "Worker {} found invalid range {:?} for chunk {} (data len: {}) in pad {}",
                            worker_id, range, chunk_index, context.base_context.data.len(), current_pad_address
                        );
                        return Err((
                            Error::Internal(format!(
                                "Invalid data range for chunk {} (pad {}). Range: {:?}, Data Len: {}",
                                chunk_index, current_pad_address, range, context.base_context.data.len()
                            )),
                            pad,
                        ));
                    }
                    &context.base_context.data[range.clone()]
                }
            };

            if chunk_data_slice.len() != pad.size {
                warn!(
                    "Worker {}: Pad {} (chunk {}) size mismatch. Expected: {}, Got slice len: {}. Key: {}",
                    worker_id, pad.address, pad.chunk_index, pad.size, chunk_data_slice.len(), context.base_context.name
                 );
            }

            let encoding = if is_index_pad {
                DATA_ENCODING_PUBLIC_INDEX
            } else if context.base_context.public {
                DATA_ENCODING_PUBLIC_DATA
            } else {
                DATA_ENCODING_PRIVATE_DATA
            };

            // --- Start: Retry logic for network.put ---
            let max_put_retries = 3;
            let mut last_put_error: Option<Error> = None;

            for attempt in 1..=max_put_retries {
                match context
                    .base_context
                    .network
                    .put(
                        client,
                        &pad, // Use original pad info for put attempt
                        chunk_data_slice,
                        encoding,
                        context.base_context.public,
                    )
                    .await
                {
                    Ok(_) => {
                        // Put succeeded, call callbacks and update status in the index
                        invoke_put_callback(&context.put_callback, PutEvent::PadsWritten)
                            .await
                            .map_err(|e| {
                                (
                                    Error::Internal(format!(
                                        "Callback error (PadsWritten): {:?}",
                                        e
                                    )),
                                    pad.clone(), // Return original pad on callback error
                                )
                            })?;
                        if initial_status == PadStatus::Generated {
                            invoke_put_callback(&context.put_callback, PutEvent::PadReserved)
                                .await
                                .map_err(|e| {
                                    (
                                        Error::Internal(format!(
                                            "Callback error (PadReserved): {:?}",
                                            e
                                        )),
                                        pad.clone(), // Return original pad on callback error
                                    )
                                })?;
                        }

                        // Attempt to update status immediately after successful put
                        pad_after_put = context
                            .base_context
                            .index
                            .write()
                            .await
                            .update_pad_status(
                                &context.base_context.name,
                                &current_pad_address,
                                PadStatus::Written,
                                None,
                            )
                            .map_err(|e| (e, pad.clone()))?; // Return original pad if update fails

                        // Mark as succeeded and break the retry loop
                        put_succeeded = true;
                        last_put_error = None; // Clear last error
                        break;
                    }
                    Err(e) => {
                        // Put attempt failed
                        warn!(
                            "Worker {} failed put attempt {}/{} for pad {} (chunk {}): {}. Retrying...",
                            worker_id, attempt, max_put_retries, current_pad_address, pad.chunk_index, e
                        );
                        last_put_error = Some(Error::Network(e)); // Store the error
                        if attempt < max_put_retries {
                            // Wait before retrying
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
            // --- End: Retry logic for network.put ---

            // Check if put ultimately failed after all retries
            if !put_succeeded {
                error!(
                    "Worker {} failed put for pad {} (chunk {}) after {} retries: {}",
                    worker_id,
                    current_pad_address,
                    pad.chunk_index,
                    max_put_retries,
                    last_put_error.as_ref().unwrap() // Safe unwrap: error is Some if put_succeeded is false
                );
                // Return the last encountered error and the original pad state
                return Err((last_put_error.unwrap(), pad));
            }
            // If we reach here, put_succeeded is true, and pad_after_put holds the pad with status Written.
            // The confirmation logic (if !no_verify) will run outside the 'if should_put' block.
        } else {
            // Pad status was already Written or Free, no put needed. Mark as succeeded.
            pad_after_put = pad.clone();
            put_succeeded = true;
        }

        // --- Start: Confirmation logic (only runs if put_succeeded is true) ---
        if put_succeeded {
            if *context.no_verify {
                // Update status directly to Confirmed if no verification is needed
                context
                    .base_context
                    .index
                    .write()
                    .await
                    .update_pad_status(
                        &context.base_context.name,
                        &current_pad_address,
                        PadStatus::Confirmed,
                        None,
                    )
                    .map_err(|e| (e, pad_after_put.clone()))?;

                let previous_count = context.confirmed_counter.fetch_add(1, Ordering::SeqCst);
                let current_count = previous_count + 1;
                invoke_put_callback(&context.put_callback, PutEvent::PadsConfirmed)
                    .await
                    .map_err(|e| {
                        (
                            Error::Internal(format!(
                                "Callback error (PadsConfirmed - no_verify): {:?}",
                                e
                            )),
                            pad_after_put.clone(),
                        )
                    })?;
                if current_count == context.total_pads {
                    context.completion_notifier.notify_waiters();
                }
                return Ok((pad.chunk_index, ()));
            } else {
                let start_time = Instant::now();
                loop {
                    if start_time.elapsed() >= MAX_CONFIRMATION_DURATION {
                        warn!(
                            "Worker {} failed to confirm pad {} (chunk {}) within time budget ({:?}). Will trigger recycling.",
                            worker_id, current_pad_address, pad_after_put.chunk_index, MAX_CONFIRMATION_DURATION
                        );
                        return Err((
                            Error::Timeout(format!(
                                "Confirmation timeout for pad {}",
                                current_pad_address
                            )),
                            pad_after_put,
                        ));
                    }

                    let secret_key_owned;
                    let secret_key_ref = if context.base_context.public {
                        None
                    } else {
                        secret_key_owned = pad_after_put.secret_key();
                        Some(&secret_key_owned)
                    };

                    match context
                        .base_context
                        .network
                        .get(client, &current_pad_address, secret_key_ref)
                        .await
                    {
                        Ok(gotten_pad) => {
                            if (pad_after_put.last_known_counter == 0 && gotten_pad.counter == 0)
                                || pad_after_put.last_known_counter <= gotten_pad.counter
                            {
                                context
                                    .base_context
                                    .index
                                    .write()
                                    .await
                                    .update_pad_status(
                                        &context.base_context.name,
                                        &current_pad_address,
                                        PadStatus::Confirmed,
                                        Some(gotten_pad.counter),
                                    )
                                    .map_err(|e| (e, pad_after_put.clone()))?;

                                let previous_count =
                                    context.confirmed_counter.fetch_add(1, Ordering::SeqCst);
                                let current_count = previous_count + 1;
                                invoke_put_callback(&context.put_callback, PutEvent::PadsConfirmed)
                                    .await
                                    .map_err(|e| {
                                        (
                                            Error::Internal(format!(
                                                "Callback error (PadsConfirmed): {:?}",
                                                e
                                            )),
                                            pad_after_put.clone(),
                                        )
                                    })?;

                                if current_count == context.total_pads {
                                    context.completion_notifier.notify_waiters();
                                }
                                return Ok((pad.chunk_index, ()));
                            } else {
                                warn!("Pad {} counter decreased during confirmation check ({} -> {}). Retrying check.", current_pad_address, pad_after_put.last_known_counter, gotten_pad.counter);
                            }
                        }
                        Err(e) => {
                            warn!("Worker {} network.get failed during confirmation for pad {}: {}. Retrying check.", worker_id, current_pad_address, e);
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
        // --- End: Confirmation logic ---

        // This path should theoretically not be reached if logic is correct.
        // It implies put_succeeded remained false without returning an error from the retry loop.
        error!(
            "Worker {} reached unexpected end of process function for pad {}",
            worker_id, current_pad_address
        );
        Err((
            Error::Internal(format!(
                "Reached unexpected end of process function for pad {}",
                current_pad_address
            )),
            pad_after_put, // Use the state after potential put attempt
        ))
    }
}

async fn write_pipeline(
    context: Context,
    pads: Vec<PadInfo>,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<(), Error> {
    let key_name = context.name.clone();
    let total_pads = context.chunk_ranges.len();

    if total_pads == 0 {
        return Ok(());
    }

    let mut worker_txs = Vec::with_capacity(WORKER_COUNT);
    let mut worker_rxs = Vec::with_capacity(WORKER_COUNT);
    for _ in 0..WORKER_COUNT {
        let (tx, rx) = bounded::<PadInfo>(total_pads.saturating_add(1) / WORKER_COUNT + BATCH_SIZE);
        worker_txs.push(tx);
        worker_rxs.push(rx);
    }
    let (global_tx, global_rx) = bounded::<PadInfo>(total_pads + WORKER_COUNT * BATCH_SIZE);

    let (recycle_tx, recycle_rx) =
        bounded::<(Error, PadInfo)>(total_pads * PAD_RECYCLING_RETRIES + WORKER_COUNT);

    let initial_confirmed_count = pads
        .iter()
        .filter(|p| p.status == PadStatus::Confirmed)
        .count();

    let initial_chunks_to_reserve = pads
        .iter()
        .filter(|p| p.status == PadStatus::Generated)
        .count();

    let confirmed_pads_counter = Arc::new(AtomicUsize::new(initial_confirmed_count));

    if initial_confirmed_count == total_pads {
        invoke_put_callback(&put_callback, PutEvent::Complete)
            .await
            .unwrap();
        return Ok(());
    }

    let completion_notifier = Arc::new(Notify::new());
    let total_pads_atomic = Arc::new(AtomicUsize::new(total_pads));

    let mut clients = Vec::with_capacity(WORKER_COUNT);
    for worker_id in 0..WORKER_COUNT {
        let client = context.network.get_client(Config::Put).await.map_err(|e| {
            error!("Failed to get client for worker {}: {}", worker_id, e);
            Error::Network(NetworkError::ClientAccessError(format!(
                "Failed to get client for worker {}: {}",
                worker_id, e
            )))
        })?;
        clients.push(Arc::new(client));
    }

    let put_task_context = Arc::new(PutTaskContext {
        base_context: context.clone(),
        no_verify: Arc::new(no_verify),
        put_callback: put_callback.clone(),
        total_pads,
        completion_notifier: completion_notifier.clone(),
        confirmed_counter: confirmed_pads_counter.clone(),
    });

    invoke_put_callback(
        &put_callback,
        PutEvent::Starting {
            total_chunks: total_pads,
            initial_written_count: pads
                .iter()
                .filter(|p| p.status == PadStatus::Written || p.status == PadStatus::Confirmed)
                .count(),
            initial_confirmed_count,
            chunks_to_reserve: initial_chunks_to_reserve,
        },
    )
    .await
    .unwrap();

    let send_pads_task = {
        let pads = pads.clone();
        let worker_txs_clone = worker_txs.clone();
        let global_tx_clone = global_tx.clone();
        let completion_notifier_clone = completion_notifier.clone();
        let confirmed_pads_counter_clone = confirmed_pads_counter.clone();
        let total_pads_atomic_clone = total_pads_atomic.clone();

        tokio::spawn(async move {
            let mut worker_index = 0;

            let mut unconfirmed_count = 0;
            for pad in pads {
                if pad.status != PadStatus::Confirmed {
                    unconfirmed_count += 1;
                    let target_tx = &worker_txs_clone[worker_index % WORKER_COUNT];
                    if let Err(_e) = target_tx.send(pad.clone()).await {
                        break;
                    }
                    worker_index += 1;
                }
            }

            loop {
                let confirmed_count = confirmed_pads_counter_clone.load(Ordering::SeqCst);
                let total_count = total_pads_atomic_clone.load(Ordering::SeqCst);

                if confirmed_count >= total_count {
                    break;
                }

                tokio::select! {
                    _ = completion_notifier_clone.notified() => {
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        // Continue checking
                    }
                }
            }

            for tx in worker_txs_clone {
                tx.close();
            }

            global_tx_clone.close();
        })
    };

    let recycler_task = {
        let index = context.index.clone();
        let key_name_clone = key_name.clone();
        let recycle_rx = recycle_rx.clone();
        let global_pad_tx = global_tx.clone();
        let completion_notifier_clone = completion_notifier.clone();
        let confirmed_pads_counter_clone = confirmed_pads_counter.clone();
        let total_pads_for_recycler = total_pads;

        tokio::spawn(async move {
            let mut recycled_count = 0;
            let max_recycles = total_pads_for_recycler * PAD_RECYCLING_RETRIES;

            loop {
                if confirmed_pads_counter_clone.load(Ordering::SeqCst) >= total_pads_for_recycler {
                    break;
                }

                tokio::select! {
                    biased;
                    _ = completion_notifier_clone.notified() => {
                        break;
                    },
                    recv_result = recycle_rx.recv() => {
                        match recv_result {
                            Ok((_error_cause, pad_to_recycle)) => {
                                recycled_count += 1;
                                if recycled_count > max_recycles {
                                    continue;
                                }

                                match index.write().await.recycle_errored_pad(&key_name_clone, &pad_to_recycle.address).await {
                                    Ok(new_pad) => {
                                        if let Err(_send_err) = global_pad_tx.send(new_pad).await {
                                            break;
                                        }
                                    },
                                    Err(recycle_err) => {
                                        error!(
                                            "Failed to recycle pad {} for key {}: {}",
                                            pad_to_recycle.address,
                                            key_name_clone,
                                            recycle_err
                                        );
                                    }
                                }
                            }
                            Err(_) => {
                                break;
                            }
                        }
                    }
                }
            }
            global_pad_tx.close();
        })
    };

    let pool: WorkerPool<
        PadInfo,
        PutTaskContext,
        Object<ClientManager>,
        PutTaskProcessor,
        (),
        Error,
    > = WorkerPool::new(
        WORKER_COUNT,
        crate::ops::BATCH_SIZE,
        put_task_context,
        Arc::new(PutTaskProcessor),
        clients,
        worker_rxs,
        global_rx,
        Some(recycle_tx),
        completion_notifier.clone(),
        Some(total_pads_atomic),
        Some(confirmed_pads_counter.clone()),
    );

    let pool_result = pool.run().await;

    if let Err(_e) = send_pads_task.await {}

    if let Err(_e) = recycler_task.await {
    } else {
    }

    match pool_result {
        Ok(_results) => {
            let final_confirmed_count = confirmed_pads_counter.load(Ordering::SeqCst);
            if final_confirmed_count != total_pads {}
        }
        Err(pool_error) => {
            completion_notifier.notify_waiters();
            let final_error = match pool_error {
                PoolError::TaskError(task_err) => task_err,
                PoolError::JoinError(_join_err) => {
                    Error::Internal("Worker task join error".to_string())
                }
                PoolError::PoolSetupError(msg) => {
                    Error::Internal(format!("Pool setup error: {}", msg))
                }
                PoolError::WorkerError(msg) => Error::Internal(format!("Worker error: {}", msg)),
            };
            return Err(final_error);
        }
    }

    let final_confirmed_count = confirmed_pads_counter.load(Ordering::SeqCst);
    if final_confirmed_count != total_pads {
        return Err(Error::Internal(format!(
            "Put operation finished for key {}, but final confirmed count ({}) does not match total pads ({}).",
            key_name, final_confirmed_count, total_pads
        )));
    }

    let _final_pad_status = pads.last().map(|p| p.status);
    invoke_put_callback(&put_callback, PutEvent::Complete)
        .await
        .unwrap();

    Ok(())
}
