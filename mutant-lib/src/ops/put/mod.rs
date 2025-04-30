use crate::error::Error;
use crate::index::{PadInfo, PadStatus};
use crate::internal_events::invoke_put_callback;
use crate::network::client::{ClientManager, Config};
use crate::network::{Network, NetworkError};
use crate::ops::{utils::*, BATCH_SIZE, MAX_CONFIRMATION_DURATION, WORKER_COUNT};
use async_channel::{bounded, Receiver, Sender};
use autonomi::ScratchpadAddress;
use deadpool::managed::Object;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, error, info, warn};
use mutant_protocol::{PutCallback, PutEvent, StorageMode};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{Notify, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{
    DATA_ENCODING_PRIVATE_DATA, DATA_ENCODING_PUBLIC_DATA, DATA_ENCODING_PUBLIC_INDEX,
    PAD_RECYCLING_RETRIES,
};

pub(super) async fn put(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    content: &[u8],
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    if index.read().await.contains_key(&key_name) {
        if index
            .read()
            .await
            .verify_checksum(&key_name, content, mode.clone())
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
            index.write().await.remove_key(&key_name).unwrap();
            return first_store(
                index,
                network,
                key_name,
                content,
                mode,
                public,
                no_verify,
                put_callback,
            )
            .await;
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
    data_bytes: &[u8],
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

    let chunks = index.read().await.chunk_data(data_bytes, mode, public);

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        chunks: Arc::new(chunks),
    };

    write_pipeline(context, pads.clone(), public, no_verify, put_callback).await;

    Ok(pads[0].address)
}

async fn first_store(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: &[u8],
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let (pads, chunks) = index
        .write()
        .await
        .create_key(name, data_bytes, mode, public)?;

    info!("Created key {} with {} pads", name, pads.len());

    let address = pads[0].address;

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        chunks: Arc::new(chunks),
    };

    write_pipeline(context, pads.clone(), public, no_verify, put_callback).await;

    Ok(address)
}

async fn write_pipeline(
    context: Context,
    pads: Vec<PadInfo>,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) {
    info!("Writing pipeline for key {}", context.name);
    let total_pads = context.chunks.len();
    let (pad_tx, pad_rx) = bounded::<PadInfo>(total_pads + WORKER_COUNT * BATCH_SIZE);

    let initial_confirmed_count = pads
        .iter()
        .filter(|p| p.status == PadStatus::Confirmed)
        .count();

    let initial_chunks_to_reserve = pads
        .iter()
        .filter(|p| p.status == PadStatus::Generated)
        .count();

    info!("Initial confirmed count: {}", initial_confirmed_count);
    info!("Initial chunks to reserve: {}", initial_chunks_to_reserve);

    let process_future = process_pads(
        context.name.clone(),
        Arc::new(no_verify),
        context.clone(),
        pad_tx.clone(),
        pad_rx,
        initial_confirmed_count,
        initial_chunks_to_reserve,
        public,
        put_callback.clone(),
    );

    info!("Sending pads to pipeline");

    let send_pads_task = {
        let pads = pads.clone();
        let pad_tx = pad_tx.clone();
        async move {
            for pad in pads {
                if pad.status != PadStatus::Confirmed {
                    if let Err(e) = pad_tx.send(pad.clone()).await {
                        error!(
                            "Failed to send initial pad {} to channel: {}",
                            pad.address, e
                        );
                    }
                }
            }
            pad_tx.close();
        }
    };
    tokio::spawn(send_pads_task);

    info!(
        "Awaiting pad processing completion for key {}",
        context.name
    );
    if let Err(e) = process_future.await {
        error!("Error processing pads for key {}: {:?}", context.name, e);
    }
    info!("Pad processing finished for key {}", context.name);
}

async fn process_pads(
    key_name: Arc<String>,
    no_verify: Arc<bool>,
    context: Context,
    pad_tx: Sender<PadInfo>,
    pad_rx: Receiver<PadInfo>,
    initial_confirmed_count: usize,
    initial_chunks_to_reserve: usize,
    public: bool,
    put_callback: Option<PutCallback>,
) -> Result<(), Error> {
    info!("Starting pad processing manager for {}", key_name);

    let total_pads = context.chunks.len();
    let confirmed_pads = Arc::new(AtomicUsize::new(initial_confirmed_count));

    if initial_confirmed_count == total_pads {
        info!(
            "All {} pads for key {} already confirmed. Skipping pipeline.",
            total_pads, key_name
        );
        return Ok(());
    }

    invoke_put_callback(
        &put_callback,
        PutEvent::Starting {
            total_chunks: total_pads,
            initial_written_count: 0,
            initial_confirmed_count: initial_confirmed_count,
            chunks_to_reserve: initial_chunks_to_reserve,
        },
    )
    .await
    .unwrap();

    let mut workers = FuturesUnordered::new();
    let completion_notifier = Arc::new(Notify::new());

    for i in 0..WORKER_COUNT {
        let worker_context = context.clone();
        let worker_confirmed_counter = confirmed_pads.clone();
        let worker_pad_rx = pad_rx.clone();
        let worker_pad_tx = pad_tx.clone();
        let worker_put_callback = put_callback.clone();
        let worker_no_verify = no_verify.clone();
        let worker_key_name = key_name.clone();
        let worker_total_pads = total_pads;
        let worker_completion_notifier = completion_notifier.clone();

        workers.push(tokio::spawn(async move {
            info!("Spawning worker {} for key {}", i, worker_key_name);
            pad_processing_worker_semaphore(
                i,
                worker_context,
                worker_confirmed_counter,
                worker_pad_rx,
                worker_pad_tx,
                public,
                worker_no_verify,
                worker_put_callback,
                worker_total_pads,
                worker_completion_notifier,
            )
            .await
        }));
    }

    info!(
        "Waiting for {} workers to complete for key {}",
        WORKER_COUNT, key_name
    );

    let mut task_handles = FuturesUnordered::new();

    while let Some(result) = workers.next().await {
        match result {
            Ok(Ok(worker_task_handles)) => {
                for handle in worker_task_handles {
                    task_handles.push(handle);
                }
            }
            Ok(Err(e)) => {
                error!(
                    "Worker main loop finished with error for key {}: {}",
                    key_name, e
                )
            }
            Err(e) => error!("Worker main loop panicked for key {}: {:?}", key_name, e),
        }
    }

    info!(
        "All {} worker main loops finished for key {}. Waiting for {} spawned pad tasks...",
        WORKER_COUNT,
        key_name,
        task_handles.len()
    );

    while let Some(task_result) = task_handles.next().await {
        match task_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!(
                    "Spawned pad task finished with error for key {}: {}",
                    key_name, e
                );
            }
            Err(e) => {
                error!("Spawned pad task panicked for key {}: {:?}", key_name, e);
            }
        }
    }

    info!("All spawned pad tasks finished for key {}", key_name);

    let final_confirmed_count = confirmed_pads.load(Ordering::SeqCst);
    if final_confirmed_count != total_pads {
        warn!(
            "Pad processing finished for key {}, but not all pads confirmed ({} / {}).",
            key_name, final_confirmed_count, total_pads
        );
    }

    invoke_put_callback(&put_callback, PutEvent::Complete)
        .await
        .unwrap();

    Ok(())
}

async fn pad_processing_worker_semaphore(
    worker_id: usize,
    context: Context,
    confirmed_counter: Arc<AtomicUsize>,
    pad_rx: Receiver<PadInfo>,
    pad_tx: Sender<PadInfo>,
    public: bool,
    no_verify: Arc<bool>,
    put_callback: Option<PutCallback>,
    total_pads: usize,
    completion_notifier: Arc<Notify>,
) -> Result<Vec<JoinHandle<Result<(), Error>>>, Error> {
    info!(
        "Worker {} (Semaphore) acquiring client for key {}",
        worker_id, context.name
    );
    let client_guard = Arc::new(context.network.get_client(Config::Put).await.map_err(|e| {
        error!("Worker {} failed to get client: {}", worker_id, e);
        Error::Network(NetworkError::ClientAccessError(format!(
            "Worker {} failed to get client: {}",
            worker_id, e
        )))
    })?);
    info!(
        "Worker {} (Semaphore) acquired client for key {}",
        worker_id, context.name
    );

    let semaphore = Arc::new(Semaphore::new(BATCH_SIZE));
    let key_name = context.name.clone();
    let worker_pad_tx = pad_tx;

    let mut handles: Vec<JoinHandle<Result<(), Error>>> = Vec::new();

    loop {
        if confirmed_counter.load(Ordering::SeqCst) >= total_pads {
            info!(
                "Worker {} detected completion. Notifying and exiting.",
                worker_id
            );
            completion_notifier.notify_waiters();
            break;
        }

        let pad_option = tokio::select! {
            biased;
            _ = completion_notifier.notified() => {
                info!("Worker {} notified of completion while waiting for pad. Exiting.", worker_id);
                None::<PadInfo>
            },
            recv_result = pad_rx.recv() => {
                match recv_result {
                    Ok(pad) => {
                        debug!("Worker {} received pad {} from channel.", worker_id, pad.address);
                        Some(pad)
                    },
                    Err(_) => {
                        info!("Worker {} pad channel closed. Exiting.", worker_id);
                        None::<PadInfo>
                    }
                }
            }
        };

        match pad_option {
            Some(pad) => {
                debug!(
                    "Worker {} attempting to acquire permit for pad {} ({:?}).",
                    worker_id, pad.address, pad.status
                );

                let permit = tokio::select! {
                    biased;
                    _ = completion_notifier.notified() => {
                        info!("Worker {} notified of completion while waiting for permit. Dropping pad {} and exiting.", worker_id, pad.address);
                        continue;
                    },
                    permit_result = semaphore.clone().acquire_owned() => {
                        match permit_result {
                            Ok(p) => p,
                            Err(_) => {
                                info!("Worker {} semaphore closed while acquiring permit. Exiting.", worker_id);
                                break;
                            }
                        }
                    }
                };
                debug!(
                    "Worker {} acquired permit for pad {}.",
                    worker_id, pad.address
                );

                let task_context = context.clone();
                let task_confirmed_counter = confirmed_counter.clone();
                let task_pad_tx = worker_pad_tx.clone();
                let task_no_verify = no_verify.clone();
                let task_put_callback = put_callback.clone();
                let task_client_guard = client_guard.clone();
                let task_key_name = key_name.clone();
                let task_completion_notifier = completion_notifier.clone();
                let task_worker_id = worker_id;

                let pad_address_for_log = pad.address;

                let handle = tokio::spawn(async move {
                    let _permit_guard = permit;
                    debug!(
                        "Task (Worker {}) started processing pad {}.",
                        task_worker_id, pad_address_for_log
                    );

                    let task_result = process_single_pad_task(
                        task_worker_id,
                        task_context,
                        task_confirmed_counter,
                        pad,
                        task_pad_tx,
                        public,
                        task_no_verify,
                        task_put_callback,
                        task_client_guard,
                        task_key_name,
                        total_pads,
                        task_completion_notifier,
                    )
                    .await;

                    debug!(
                        "Task (Worker {}) finished processing pad {}, permit released.",
                        task_worker_id, pad_address_for_log
                    );
                    task_result
                });
                handles.push(handle);
            }
            None => {
                info!(
                    "Worker {} received None or completion signal from pad channel select. Exiting loop.",
                    worker_id
                );
                break;
            }
        }
    }

    info!(
        "Worker {} (Semaphore) main loop finished for key {}",
        worker_id, context.name
    );
    Ok(handles)
}

async fn process_single_pad_task(
    worker_id: usize,
    context: Context,
    confirmed_counter: Arc<AtomicUsize>,
    pad: PadInfo,
    pad_tx: Sender<PadInfo>,
    public: bool,
    no_verify: Arc<bool>,
    put_callback: Option<PutCallback>,
    client_guard: Arc<Object<ClientManager>>,
    key_name: Arc<String>,
    total_pads: usize,
    completion_notifier: Arc<Notify>,
) -> Result<(), Error> {
    let result = async {
        let client = &*client_guard;
        let current_pad_address = pad.address;
        let initial_status = pad.status;

        let recycle_context_index = context.index.clone();
        let recycle_key_name = key_name.clone();
        let recycle_pad_tx = pad_tx.clone();

        let recycle_pad = |pad_to_recycle: PadInfo| async move {
            warn!(
                "Worker {} attempting to recycle pad {} for key {}",
                worker_id,
                pad_to_recycle.address,
                recycle_key_name
            );
            match recycle_context_index
                .write()
                .await
                .recycle_errored_pad(&recycle_key_name, &pad_to_recycle.address)
            {
                Ok(new_pad) => {
                    if let Err(send_err) = recycle_pad_tx.send(new_pad).await {
                        error!(
                            "Worker {} failed to send recycled pad {} back to queue: {}",
                            worker_id,
                            pad_to_recycle.address,
                            send_err
                        );
                    }
                }
                Err(recycle_err) => {
                    error!(
                        "Worker {} failed to recycle pad {}: {}",
                        worker_id,
                        pad_to_recycle.address,
                        recycle_err
                    );
                }
            }
        };

        let mut pad_after_put = pad.clone();
        let mut put_succeeded = initial_status == PadStatus::Written;

        if initial_status == PadStatus::Generated || initial_status == PadStatus::Free {
            let chunk_data = match context.chunks.get(pad.chunk_index as usize) {
                Some(data) => data.clone(),
                None => {
                    error!(
                        "CRITICAL (Worker {}): Chunk index {} out of bounds for key {}. Pad {}. Cannot process.",
                        worker_id,
                        pad.chunk_index,
                        key_name,
                        pad.address
                    );
                    return Ok::<(), Error>(());
                }
            };

            let is_index = public && pad.chunk_index == 0 && total_pads > 1;
            let encoding = if is_index {
                DATA_ENCODING_PUBLIC_INDEX
            } else if public {
                DATA_ENCODING_PUBLIC_DATA
            } else {
                DATA_ENCODING_PRIVATE_DATA
            };

            let mut retries_left = PAD_RECYCLING_RETRIES;
            loop {
                match context
                    .network
                    .put(client, &pad, &chunk_data, encoding, public)
                    .await
                {
                    Ok(_) => {
                        invoke_put_callback(&put_callback, PutEvent::PadsWritten)
                            .await
                            .unwrap();
                        if initial_status == PadStatus::Generated {
                            invoke_put_callback(&put_callback, PutEvent::PadReserved)
                                .await
                                .unwrap();
                        }

                        match context.index.write().await.update_pad_status(
                            &key_name,
                            &current_pad_address,
                            PadStatus::Written,
                            None,
                        ) {
                            Ok(updated_pad) => {
                                pad_after_put = updated_pad;
                                put_succeeded = true;
                                break;
                            }
                            Err(e) => {
                                error!(
                                    "Worker {} failed to update pad {} status to Written: {}. Pad might be orphaned.",
                                    worker_id,
                                    current_pad_address,
                                    e
                                );
                                put_succeeded = false;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Worker {} failed put for pad {} (chunk {}): {}. Retries left: {}",
                            worker_id,
                            current_pad_address,
                            pad.chunk_index,
                            e,
                            retries_left
                        );
                        retries_left -= 1;
                        if retries_left == 0 {
                            recycle_pad(pad.clone()).await;
                            return Ok::<(), Error>(());
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }

        if put_succeeded {
            if *no_verify {
                match context.index.write().await.update_pad_status(
                    &key_name,
                    &current_pad_address,
                    PadStatus::Confirmed,
                    None,
                ) {
                    Ok(_) => {
                        let previous_count = confirmed_counter.fetch_add(1, Ordering::SeqCst);
                        let current_count = previous_count + 1;
                        invoke_put_callback(&put_callback, PutEvent::PadsConfirmed)
                            .await
                            .unwrap();
                        debug!(
                            "Worker {} marked Pad {} as Confirmed (no_verify). Total Confirmed: {}",
                            worker_id,
                            current_pad_address,
                            previous_count + 1
                        );
                        if current_count == total_pads {
                            info!(
                                "Worker {} marked LAST Pad {} as Confirmed (no_verify). Notifying.",
                                worker_id,
                                current_pad_address
                            );
                            completion_notifier.notify_waiters();
                        }
                    }
                    Err(e) => {
                        error!(
                            "Worker {} failed to update pad {} status to Confirmed (no_verify): {}",
                            worker_id,
                            current_pad_address,
                            e
                        );
                    }
                }
            } else {
                let start_time = Instant::now();
                loop {
                    if start_time.elapsed() >= MAX_CONFIRMATION_DURATION {
                        warn!(
                            "Worker {} failed to confirm pad {} (chunk {}) within time budget ({:?}). Recycling.",
                            worker_id,
                            current_pad_address,
                            pad_after_put.chunk_index,
                            MAX_CONFIRMATION_DURATION
                        );
                        recycle_pad(pad_after_put.clone()).await;
                        return Ok::<(), Error>(());
                    }

                    let secret_key_owned;
                    let secret_key_ref = if public {
                        None
                    } else {
                        secret_key_owned = pad_after_put.secret_key();
                        Some(&secret_key_owned)
                    };

                    match context
                        .network
                        .get(client, &current_pad_address, secret_key_ref)
                        .await
                    {
                        Ok(gotten_pad) => {
                            if (pad_after_put.last_known_counter == 0
                                && gotten_pad.counter == 0)
                                || pad_after_put.last_known_counter <= gotten_pad.counter
                            {
                                match context.index.write().await.update_pad_status(
                                    &key_name,
                                    &current_pad_address,
                                    PadStatus::Confirmed,
                                    Some(gotten_pad.counter),
                                ) {
                                    Ok(_) => {
                                        let previous_count =
                                            confirmed_counter.fetch_add(1, Ordering::SeqCst);
                                        let current_count = previous_count + 1;
                                        invoke_put_callback(
                                            &put_callback,
                                            PutEvent::PadsConfirmed,
                                        )
                                        .await
                                        .unwrap();
                                        debug!(
                                            "Worker {} confirmed Pad {}. Total Confirmed: {}",
                                            worker_id,
                                            current_pad_address,
                                            previous_count + 1
                                        );
                                        if current_count == total_pads {
                                            info!(
                                                "Worker {} confirmed the LAST pad {}. Notifying waiters.",
                                                worker_id,
                                                current_pad_address
                                            );
                                            completion_notifier.notify_waiters();
                                        }
                                        return Ok::<(), Error>(());
                                    }
                                    Err(e) => {
                                        error!(
                                            "Worker {} failed to update pad {} status to Confirmed after get: {}. Retrying confirm loop.",
                                            worker_id,
                                            current_pad_address,
                                            e
                                        );
                                    }
                                }
                            } else {
                                debug!(
                                    "Worker {} Pad {} counter check failed (last_known: {}, gotten: {}). Retrying get.",
                                    worker_id,
                                    current_pad_address,
                                    pad_after_put.last_known_counter,
                                    gotten_pad.counter
                                );
                            }
                        }
                        Err(e) => {
                            debug!(
                                "Worker {} error getting pad {} for confirmation: {}. Retrying get.",
                                worker_id,
                                current_pad_address,
                                e
                            );
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        } else {
            warn!(
                "Worker {} skipping confirmation for pad {} because put did not succeed.",
                worker_id,
                current_pad_address
            );
        }
        Ok::<(), Error>(())
    }
    .await;

    if let Err(e) = &result {
        error!(
            "Worker {} error processing pad {}: {}",
            worker_id, pad.address, e
        );
    }
    result
}
