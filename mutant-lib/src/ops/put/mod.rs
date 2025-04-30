use crate::error::Error;
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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{Notify, RwLock};
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
    chunks: Arc<Vec<Vec<u8>>>,
    public: bool,
}

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
    if index.read().await.contains_key(key_name) {
        if index
            .read()
            .await
            .verify_checksum(key_name, content, mode.clone())
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
        public,
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback).await?;

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
        public,
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
    client_manager: Arc<Object<ClientManager>>,
    completion_notifier: Arc<Notify>,
    confirmed_counter: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct PutTaskProcessor;

#[async_trait]
impl AsyncTask<PadInfo, PutTaskContext, (), Error> for PutTaskProcessor {
    type ItemId = usize;

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<PutTaskContext>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, ()), (Error, PadInfo)> {
        let client = &*context.client_manager;
        let current_pad_address = pad.address;
        let initial_status = pad.status;

        let mut pad_after_put = pad.clone();
        let mut put_succeeded = initial_status == PadStatus::Written;

        if initial_status == PadStatus::Generated || initial_status == PadStatus::Free {
            let chunk_data = match context.base_context.chunks.get(pad.chunk_index as usize) {
                Some(data) => data.clone(),
                None => {
                    error!(
                        "CRITICAL (Worker {}): Chunk index {} out of bounds for key {}. Pad {}. Cannot process.",
                        worker_id, pad.chunk_index, context.base_context.name, pad.address
                    );
                    return Err((
                        Error::Internal(format!(
                            "Chunk index out of bounds for pad {}",
                            pad.address
                        )),
                        pad,
                    ));
                }
            };

            let is_index =
                context.base_context.public && pad.chunk_index == 0 && context.total_pads > 1;
            let encoding = if is_index {
                DATA_ENCODING_PUBLIC_INDEX
            } else if context.base_context.public {
                DATA_ENCODING_PUBLIC_DATA
            } else {
                DATA_ENCODING_PRIVATE_DATA
            };

            match context
                .base_context
                .network
                .put(
                    client,
                    &pad,
                    &chunk_data,
                    encoding,
                    context.base_context.public,
                )
                .await
            {
                Ok(_) => {
                    invoke_put_callback(&context.put_callback, PutEvent::PadsWritten)
                        .await
                        .map_err(|e| {
                            (
                                Error::Internal(format!("Callback error (PadsWritten): {:?}", e)),
                                pad.clone(),
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
                                    pad.clone(),
                                )
                            })?;
                    }

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
                        .map_err(|e| (e, pad.clone()))?;
                    put_succeeded = true;
                }
                Err(e) => {
                    error!(
                        "Worker {} failed put for pad {} (chunk {}): {}",
                        worker_id, current_pad_address, pad.chunk_index, e
                    );
                    return Err((Error::Network(e), pad));
                }
            }
        }

        if put_succeeded {
            if *context.no_verify {
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

                                debug!(
                                    "Worker {} confirmed Pad {}. Total Confirmed: {}",
                                    worker_id, current_pad_address, current_count
                                );

                                if current_count == context.total_pads {
                                    context.completion_notifier.notify_waiters();
                                }
                                return Ok((pad.chunk_index, ()));
                            } else {
                                warn!("Pad {} counter decreased during confirmation check ({} -> {}). Retrying check.", current_pad_address, pad_after_put.last_known_counter, gotten_pad.counter);
                            }
                        }
                        Err(e) => {
                            warn!("Worker {} failed GET during confirmation for pad {}: {}. Retrying check.", worker_id, current_pad_address, e);
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        } else {
            error!("Worker {} reached unexpected state for pad {} (Put skipped/failed, but no error returned earlier).", worker_id, current_pad_address);
            return Err((
                Error::Internal(format!(
                    "Inconsistent state for pad {}",
                    current_pad_address
                )),
                pad,
            ));
        }

        Ok((pad.chunk_index, ()))
    }
}

async fn write_pipeline(
    context: Context,
    pads: Vec<PadInfo>,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<(), Error> {
    let key_name = context.name.clone();
    info!("Writing pipeline for key {}", key_name);
    let total_pads = context.chunks.len();

    if total_pads == 0 {
        info!("No pads to process for key {}", key_name);
        return Ok(());
    }

    let (process_tx, process_rx) = bounded::<PadInfo>(total_pads + WORKER_COUNT * BATCH_SIZE);
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

    info!("Initial confirmed count: {}", initial_confirmed_count);
    info!("Initial chunks to reserve: {}", initial_chunks_to_reserve);

    let confirmed_pads_counter = Arc::new(AtomicUsize::new(initial_confirmed_count));

    if initial_confirmed_count == total_pads {
        info!(
            "All {} pads for key {} already confirmed. Skipping pipeline.",
            total_pads, key_name
        );
        invoke_put_callback(&put_callback, PutEvent::Complete)
            .await
            .unwrap();
        return Ok(());
    }

    let completion_notifier = Arc::new(Notify::new());
    let total_pads_atomic = Arc::new(AtomicUsize::new(total_pads));

    let client_guard = Arc::new(context.network.get_client(Config::Put).await.map_err(|e| {
        error!("Failed to get client for pool: {}", e);
        Error::Network(NetworkError::ClientAccessError(format!(
            "Failed to get client for pool: {}",
            e
        )))
    })?);

    let put_task_context = Arc::new(PutTaskContext {
        base_context: context.clone(),
        no_verify: Arc::new(no_verify),
        put_callback: put_callback.clone(),
        total_pads,
        client_manager: client_guard,
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
        let pad_tx = process_tx.clone();
        tokio::spawn(async move {
            for pad in pads {
                if pad.status != PadStatus::Confirmed {
                    if let Err(e) = pad_tx.send(pad.clone()).await {
                        error!(
                            "Failed to send initial pad {} to channel: {}",
                            pad.address, e
                        );
                        break;
                    }
                }
            }
            pad_tx.close();
        })
    };

    let recycler_task = {
        let index = context.index.clone();
        let key_name_clone = key_name.clone();
        let recycle_rx = recycle_rx.clone();
        let pad_tx = process_tx.clone();
        let completion_notifier_clone = completion_notifier.clone();
        let confirmed_pads_counter_clone = confirmed_pads_counter.clone();

        tokio::spawn(async move {
            info!("Starting recycler task for key {}", key_name_clone);
            let mut recycled_count = 0;
            loop {
                if confirmed_pads_counter_clone.load(Ordering::SeqCst) >= total_pads {
                    info!(
                        "Recycler detected completion for key {}. Exiting.",
                        key_name_clone
                    );
                    break;
                }

                tokio::select! {
                    biased;
                    _ = completion_notifier_clone.notified() => {
                        info!("Recycler received completion notification for key {}. Exiting.", key_name_clone);
                        break;
                    },
                    recv_result = recycle_rx.recv() => {
                        match recv_result {
                            Ok((error_cause, pad_to_recycle)) => {
                                 warn!(
                                     "Recycling pad {} for key {} due to error: {:?}",
                                     pad_to_recycle.address, key_name_clone, error_cause
                                 );
                                 recycled_count += 1;
                                 if recycled_count > total_pads * PAD_RECYCLING_RETRIES {
                                      error!("Exceeded maximum recycling attempts for key {}. Aborting recycling.", key_name_clone);
                                      break;
                                 }

                                 match index.write().await.recycle_errored_pad(&key_name_clone, &pad_to_recycle.address).await {
                                     Ok(new_pad) => {
                                         info!("Successfully recycled pad {} -> {} for key {}", pad_to_recycle.address, new_pad.address, key_name_clone);
                                         if let Err(send_err) = pad_tx.send(new_pad).await {
                                             error!("Failed to send recycled pad {} back to processing queue: {}", key_name_clone, send_err);
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
                                info!("Recycle channel closed for key {}. Exiting recycler.", key_name_clone);
                                break;
                            }
                        }
                    }
                }
            }
            info!("Recycler task finished for key {}", key_name_clone);
        })
    };

    let pool = WorkerPool::new(
        WORKER_COUNT,
        crate::ops::BATCH_SIZE,
        put_task_context,
        Arc::new(PutTaskProcessor),
        process_rx,
        Some(recycle_tx),
        completion_notifier.clone(),
        Some(total_pads_atomic),
        Some(confirmed_pads_counter.clone()),
    );

    info!("Starting worker pool for key {}", key_name);
    let pool_result = pool.run().await;

    if let Err(e) = send_pads_task.await {
        error!(
            "Initial pad sending task panicked for key {}: {:?}",
            key_name, e
        );
    }

    if let Err(e) = recycler_task.await {
        error!("Recycler task panicked for key {}: {:?}", key_name, e);
    } else {
        info!("Recycler task joined for key {}", key_name);
    }

    match pool_result {
        Ok(results) => {
            info!("Worker pool run completed successfully for key {}. Processed results for {} items.", key_name, results.len());
            let final_confirmed_count = confirmed_pads_counter.load(Ordering::SeqCst);
            if final_confirmed_count != total_pads {
                warn!(
                      "Pad processing finished for key {}, but not all pads confirmed ({} / {}). Some pads might have been lost during recycling.",
                      key_name, final_confirmed_count, total_pads
                  );
            }
        }
        Err(pool_error) => {
            error!(
                "Worker pool run failed for key {}: {:?}",
                key_name, pool_error
            );
            completion_notifier.notify_waiters();
            let final_error = match pool_error {
                PoolError::TaskError(task_err) => task_err,
                PoolError::JoinError(join_err) => {
                    Error::Internal(format!("Worker task join error: {:?}", join_err))
                }
                PoolError::PoolSetupError(msg) => {
                    Error::Internal(format!("Pool setup error: {}", msg))
                }
                PoolError::SemaphoreClosed => {
                    Error::Internal("Worker semaphore closed unexpectedly".to_string())
                }
            };
            return Err(final_error);
        }
    }

    let final_confirmed_count = confirmed_pads_counter.load(Ordering::SeqCst);
    if final_confirmed_count == total_pads {
        invoke_put_callback(&put_callback, PutEvent::Complete)
            .await
            .unwrap();
        info!("Pad processing fully completed for key {}", key_name);
    } else {
        warn!(
            "Pad processing finished for key {} with incomplete confirmation ({} / {}).",
            key_name, final_confirmed_count, total_pads
        );
    }

    Ok(())
}
