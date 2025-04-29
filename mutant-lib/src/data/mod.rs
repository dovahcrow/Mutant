use crate::network::client::{ClientManager, Config};
use crate::{
    events::{GetCallback, GetEvent, PurgeCallback, PurgeEvent, SyncCallback, SyncEvent},
    index::{master_index::MasterIndex, PadInfo, PadStatus},
    internal_events::{
        invoke_get_callback, invoke_health_check_callback, invoke_purge_callback,
        invoke_put_callback, invoke_sync_callback,
    },
    network::{Network, NetworkError},
};
use ant_networking::GetRecordError;
use autonomi::{Client, ScratchpadAddress};
use blsttc::SecretKey;
use deadpool::managed::Object;
use futures::stream::FuturesUnordered;
use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    SinkExt, StreamExt,
};
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio::sync::Semaphore;
use tokio::time::{timeout, Instant};

use mutant_protocol::{
    HealthCheckCallback, HealthCheckEvent, HealthCheckResult, PurgeResult, PutCallback, PutEvent,
    StorageMode, SyncResult,
};

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const PAD_RECYCLING_RETRIES: usize = 3;
const WORKER_COUNT: usize = 20;
const BATCH_SIZE: usize = 20;

const MAX_CONFIRMATION_DURATION: Duration = Duration::from_secs(60 * 20);

pub use crate::error::Error;

#[derive(Clone)]
struct Context {
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    name: Arc<String>,
    chunks: Arc<Vec<Vec<u8>>>,
}

pub struct Data {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
}

impl Data {
    pub fn new(network: Arc<Network>, index: Arc<RwLock<MasterIndex>>) -> Self {
        Self { network, index }
    }

    pub async fn put(
        &self,
        key_name: &str,
        content: &[u8],
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        if self.index.read().await.contains_key(&key_name) {
            if self
                .index
                .read()
                .await
                .verify_checksum(&key_name, content, mode.clone())
            {
                info!("Resume for {}", key_name);
                self.resume(key_name, content, mode, public, no_verify, put_callback)
                    .await
            } else {
                info!("Update for {}", key_name);
                self.index.write().await.remove_key(&key_name).unwrap();
                return self
                    .first_store(key_name, content, mode, public, no_verify, put_callback)
                    .await;
            }
        } else {
            info!("First store for {}", key_name);
            self.first_store(key_name, content, mode, public, no_verify, put_callback)
                .await
        }
    }

    async fn resume(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        let pads = self.index.read().await.get_pads(name);

        // check pads are in the correct mode or remove key and start over
        if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
            self.index.write().await.remove_key(name).unwrap();
            return self
                .first_store(name, data_bytes, mode, public, no_verify, put_callback)
                .await;
        }

        let chunks = self.index.read().await.chunk_data(data_bytes, mode, public);

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: Arc::new(chunks),
        };

        self.write_pipeline(context, pads.clone(), public, no_verify, put_callback)
            .await;

        Ok(pads[0].address)
    }

    async fn first_store(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) -> Result<ScratchpadAddress, Error> {
        let (pads, chunks) = self
            .index
            .write()
            .await
            .create_key(name, data_bytes, mode, public)?;

        info!("Created key {} with {} pads", name, pads.len());

        let address = pads[0].address;

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: Arc::new(chunks),
        };

        self.write_pipeline(context, pads.clone(), public, no_verify, put_callback)
            .await;

        Ok(address)
    }

    async fn write_pipeline(
        &self,
        context: Context,
        pads: Vec<PadInfo>,
        public: bool,
        no_verify: bool,
        put_callback: Option<PutCallback>,
    ) {
        info!("Writing pipeline for key {}", context.name);
        let (pad_tx, pad_rx) = unbounded::<PadInfo>();

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

        let process_future = Self::process_pads(
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

        for pad in pads {
            if pad.status != PadStatus::Confirmed {
                if let Err(e) = pad_tx.unbounded_send(pad.clone()) {
                    error!(
                        "Failed to send initial pad {} to channel: {}",
                        pad.address, e
                    );
                }
            }
        }

        info!("Dropping initial pad tx handle");
        drop(pad_tx);

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
        pad_tx: UnboundedSender<PadInfo>,
        pad_rx: UnboundedReceiver<PadInfo>,
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
        let pad_rx_arc = Arc::new(tokio::sync::Mutex::new(pad_rx));
        let completion_notifier = Arc::new(Notify::new());

        for i in 0..WORKER_COUNT {
            let worker_context = context.clone();
            let worker_confirmed_counter = confirmed_pads.clone();
            let worker_pad_rx = pad_rx_arc.clone();
            let worker_pad_tx = pad_tx.clone();
            let worker_put_callback = put_callback.clone();
            let worker_no_verify = no_verify.clone();
            let worker_key_name = key_name.clone();
            let worker_total_pads = total_pads;
            let worker_completion_notifier = completion_notifier.clone();

            workers.push(tokio::spawn(async move {
                info!("Spawning worker {} for key {}", i, worker_key_name);
                Self::pad_processing_worker_semaphore(
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
        while let Some(result) = workers.next().await {
            match result {
                Ok(Ok(())) => { /* Worker finished successfully */ }
                Ok(Err(e)) => error!("Worker finished with error for key {}: {}", key_name, e),
                Err(e) => error!("Worker panicked for key {}: {:?}", key_name, e),
            }
        }

        info!("All workers finished for key {}", key_name);

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
        pad_rx: Arc<tokio::sync::Mutex<UnboundedReceiver<PadInfo>>>,
        pad_tx: UnboundedSender<PadInfo>,
        public: bool,
        no_verify: Arc<bool>,
        put_callback: Option<PutCallback>,
        total_pads: usize,
        completion_notifier: Arc<Notify>,
    ) -> Result<(), Error> {
        info!(
            "Worker {} (Semaphore) acquiring client for key {}",
            worker_id, context.name
        );
        let client_guard =
            Arc::new(context.network.get_client(Config::Put).await.map_err(|e| {
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

        loop {
            if confirmed_counter.load(Ordering::SeqCst) >= total_pads {
                info!(
                    "Worker {} detected completion before permit/lock. Exiting.",
                    worker_id
                );
                break;
            }

            let permit = tokio::select! {
                biased;
                _ = completion_notifier.notified() => {
                    info!("Worker {} notified of completion while waiting for permit. Exiting.", worker_id);
                    return Ok(());
                },
                permit_result = semaphore.clone().acquire_owned() => {
                    match permit_result {
                        Ok(p) => p,
                        Err(_) => {
                            info!("Worker {} semaphore closed. Exiting.", worker_id);
                            return Ok(());
                        }
                    }
                }
            };
            debug!("Worker {} acquired semaphore permit.", worker_id);

            let pad_option;
            {
                let mut rx_guard = pad_rx.lock().await;

                if confirmed_counter.load(Ordering::SeqCst) >= total_pads {
                    info!(
                        "Worker {} detected completion after lock, before select. Exiting.",
                        worker_id
                    );
                    drop(permit);
                    break;
                }

                debug!(
                    "Worker {} selecting between pad receive and completion...",
                    worker_id
                );
                tokio::select! {
                    biased;
                    _ = completion_notifier.notified() => {
                        info!("Worker {} notified of completion while waiting for pad. Exiting.", worker_id);
                        pad_option = None;
                    },
                    next_result = rx_guard.next() => {
                         debug!("Worker {} received result from pad channel.", worker_id);
                         pad_option = next_result;
                    }
                }
            }

            match pad_option {
                Some(pad) => {
                    debug!(
                        "Worker {} received pad {} ({:?}), launching sub-task.",
                        worker_id, pad.address, pad.status
                    );

                    let task_permit = permit;
                    let task_context = context.clone();
                    let task_confirmed_counter = confirmed_counter.clone();
                    let task_pad_tx = worker_pad_tx.clone();
                    let task_put_callback = put_callback.clone();
                    let task_no_verify = no_verify.clone();
                    let task_client_guard = client_guard.clone();
                    let task_key_name = key_name.clone();
                    let task_completion_notifier = completion_notifier.clone();

                    tokio::spawn(async move {
                        Self::process_single_pad_task(
                            worker_id,
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
                        // Permit (task_permit) is dropped when task finishes
                        debug!("Worker {} sub-task finished, permit released.", worker_id);
                    });
                }
                None => {
                    info!(
                        "Worker {} received None or completion signal from pad channel select. Exiting loop.",
                        worker_id
                    );
                    drop(permit);
                    break;
                }
            }
        }

        info!(
            "Worker {} (Semaphore) finished processing for key {}",
            worker_id, context.name
        );
        Ok(())
    }

    async fn process_single_pad_task(
        worker_id: usize,
        context: Context,
        confirmed_counter: Arc<AtomicUsize>,
        pad: PadInfo,
        pad_tx: UnboundedSender<PadInfo>,
        public: bool,
        no_verify: Arc<bool>,
        put_callback: Option<PutCallback>,
        client_guard: Arc<Object<ClientManager>>,
        key_name: Arc<String>,
        total_pads: usize,
        completion_notifier: Arc<Notify>,
    ) -> Result<(), Error> {
        let client = &*client_guard;
        let current_pad_address = pad.address;
        let initial_status = pad.status;

        let recycle_context_index = context.index.clone();
        let recycle_key_name = key_name.clone();
        let recycle_pad_tx = pad_tx.clone();

        let recycle_pad = |pad_to_recycle: PadInfo| async move {
            warn!(
                "Worker {} attempting to recycle pad {} for key {}",
                worker_id, pad_to_recycle.address, recycle_key_name
            );
            match recycle_context_index
                .write()
                .await
                .recycle_errored_pad(&recycle_key_name, &pad_to_recycle.address)
            {
                Ok(new_pad) => {
                    if let Err(send_err) = recycle_pad_tx.unbounded_send(new_pad) {
                        error!(
                            "Worker {} failed to send recycled pad {} back to queue: {}",
                            worker_id, pad_to_recycle.address, send_err
                        );
                    }
                }
                Err(recycle_err) => {
                    error!(
                        "Worker {} failed to recycle pad {}: {}",
                        worker_id, pad_to_recycle.address, recycle_err
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
                        worker_id, pad.chunk_index, key_name, pad.address
                    );
                    return Ok(());
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
                                    worker_id, current_pad_address, e
                                );
                                put_succeeded = false;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Worker {} failed put for pad {} (chunk {}): {}. Retries left: {}",
                            worker_id, current_pad_address, pad.chunk_index, e, retries_left
                        );
                        retries_left -= 1;
                        if retries_left == 0 {
                            recycle_pad(pad.clone()).await;
                            return Ok(());
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
                                worker_id, current_pad_address
                            );
                            completion_notifier.notify_waiters();
                        }
                    }
                    Err(e) => {
                        error!(
                            "Worker {} failed to update pad {} status to Confirmed (no_verify): {}",
                            worker_id, current_pad_address, e
                        );
                    }
                }
            } else {
                let start_time = Instant::now();
                loop {
                    if start_time.elapsed() >= MAX_CONFIRMATION_DURATION {
                        warn!(
                            "Worker {} failed to confirm pad {} (chunk {}) within time budget ({:?}). Recycling.",
                            worker_id, current_pad_address, pad_after_put.chunk_index, MAX_CONFIRMATION_DURATION
                        );
                        recycle_pad(pad_after_put.clone()).await;
                        return Ok(());
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
                            if (pad_after_put.last_known_counter == 0 && gotten_pad.counter == 0)
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
                                        invoke_put_callback(&put_callback, PutEvent::PadsConfirmed)
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
                                                worker_id, current_pad_address
                                            );
                                            completion_notifier.notify_waiters();
                                        }
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        error!(
                                            "Worker {} failed to update pad {} status to Confirmed after get: {}. Retrying confirm loop.",
                                            worker_id, current_pad_address, e
                                        );
                                    }
                                }
                            } else {
                                debug!(
                                    "Worker {} Pad {} counter check failed (last_known: {}, gotten: {}). Retrying get.",
                                    worker_id, current_pad_address, pad_after_put.last_known_counter, gotten_pad.counter
                                );
                            }
                        }
                        Err(e) => {
                            debug!(
                                "Worker {} error getting pad {} for confirmation: {}. Retrying get.",
                                worker_id, current_pad_address, e
                            );
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        } else {
            warn!(
                "Worker {} skipping confirmation for pad {} because put did not succeed.",
                worker_id, current_pad_address
            );
        }
        Ok(())
    }

    async fn fetch_pads_data(
        &self,
        pads: Vec<PadInfo>,
        public: bool,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();
        let mut tasks = Vec::new();
        let client_guard = Arc::new(
            self.network
                .get_client(Config::Get)
                .await
                .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?,
        );

        for pad in pads {
            let callback_clone = get_callback.clone();
            let network = self.network.clone();
            let client_guard_clone = client_guard.clone();

            tasks.push(tokio::spawn(async move {
                let client = &*client_guard_clone;
                let mut retries_left = PAD_RECYCLING_RETRIES;
                let owned_key;
                let secret_key_ref = if public {
                    None
                } else {
                    owned_key = pad.secret_key();
                    Some(&owned_key)
                };

                let pad_data = loop {
                    match network.get(client, &pad.address, secret_key_ref).await {
                        Ok(pad_result) => {
                            if pad_result.data.len() != pad.size {
                                return Err(Error::Internal(format!(
                                    "Pad size mismatch for pad {}: expected {}, got {}",
                                    pad.address,
                                    pad.size,
                                    pad_result.data.len()
                                )));
                            }

                            invoke_get_callback(&callback_clone, GetEvent::PadsFetched)
                                .await
                                .unwrap();

                            break pad_result.data;
                        }
                        Err(e) => {
                            debug!("Error getting pad {}: {}", pad.address, e);
                            retries_left -= 1;
                            if retries_left == 0 {
                                return Err(Error::Network(e));
                            }
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                };

                Ok(pad_data)
            }));
        }

        let results = futures::future::join_all(tasks).await;

        invoke_get_callback(&get_callback, GetEvent::Complete)
            .await
            .unwrap();

        for result in results {
            match result {
                Ok(Ok(pad_data)) => data.extend_from_slice(&pad_data),
                Ok(Err(e)) => {
                    error!("Error fetching pad data during get: {:?}", e);
                    return Err(e);
                }
                Err(e) => {
                    error!("Task panic during get: {:?}", e);
                    return Err(Error::Internal(
                        "Task panic during get operation".to_string(),
                    ));
                }
            }
        }
        Ok(data)
    }

    pub async fn get_public(
        &self,
        address: &ScratchpadAddress,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        let client_guard = self
            .network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client = &*client_guard;
        let index_pad_data = self.network.get(client, address, None).await?;
        let callback = get_callback.clone();

        match index_pad_data.data_encoding {
            DATA_ENCODING_PUBLIC_INDEX => {
                let index: Vec<PadInfo> =
                    serde_cbor::from_slice(&index_pad_data.data).map_err(|e| {
                        Error::Internal(format!("Failed to decode public index: {}", e))
                    })?;

                invoke_get_callback(
                    &callback,
                    GetEvent::Starting {
                        total_chunks: index.len() + 1,
                    },
                )
                .await
                .unwrap();

                invoke_get_callback(&callback, GetEvent::PadsFetched)
                    .await
                    .unwrap();

                self.fetch_pads_data(index, true, callback).await
            }
            DATA_ENCODING_PUBLIC_DATA => Ok(index_pad_data.data),
            _ => Err(Error::Internal(format!(
                "Unexpected data encoding {} found for public address {}",
                index_pad_data.data_encoding, address
            ))),
        }
    }

    pub async fn get(
        &self,
        name: &str,
        get_callback: Option<GetCallback>,
    ) -> Result<Vec<u8>, Error> {
        if !self.index.read().await.is_finished(name) {
            return Err(Error::Internal(format!(
                "Key {} upload is not finished, cannot get data",
                name
            )));
        }

        let pads = self.index.read().await.get_pads(name);

        if pads.is_empty() {
            return Err(Error::Internal(format!("No pads found for key {}", name)));
        }

        let callback = get_callback.clone();

        invoke_get_callback(
            &callback,
            GetEvent::Starting {
                total_chunks: pads.len(),
            },
        )
        .await
        .unwrap();

        let is_public = self.index.read().await.is_public(name);

        let pads_to_fetch = if is_public && pads.len() > 1 {
            pads[1..].to_vec()
        } else {
            pads
        };

        self.fetch_pads_data(pads_to_fetch, is_public, callback)
            .await
    }

    pub async fn purge(
        &self,
        aggressive: bool,
        purge_callback: Option<PurgeCallback>,
    ) -> Result<PurgeResult, Error> {
        let pads = self.index.read().await.get_pending_pads();
        let nb_verified = Arc::new(AtomicUsize::new(0));
        let nb_failed = Arc::new(AtomicUsize::new(0));
        let callback = purge_callback.clone();

        invoke_purge_callback(
            &callback,
            PurgeEvent::Starting {
                total_count: pads.len(),
            },
        )
        .await
        .unwrap();

        let mut tasks = futures::stream::FuturesOrdered::new();
        let client_guard = self
            .network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client = Arc::new(client_guard);

        for pad in pads {
            let pad = pad.clone();
            let network = self.network.clone();
            let index = self.index.clone();
            let nb_verified_clone = nb_verified.clone();
            let nb_failed_clone = nb_failed.clone();
            let task_callback = callback.clone();
            let client_clone = client.clone();

            tasks.push_back(tokio::spawn(async move {
                let client_ref = &*client_clone;
                match network.get(client_ref, &pad.address, None).await {
                    Ok(_res) => {
                        debug!("Pad {} verified.", pad.address);
                        if let Ok(mut index_guard) = index.try_write() {
                            index_guard.verified_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for verifying pad {}", pad.address);
                        }
                        nb_verified_clone.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        if let NetworkError::GetError(GetRecordError::RecordNotFound) = e {
                            debug!("Pad {} discarded.", pad.address);
                            if let Ok(mut index_guard) = index.try_write() {
                                index_guard.discard_pending_pad(pad).unwrap();
                            } else {
                                error!("Could not acquire write lock for discarding pad {} (NotFound)", pad.address);
                            }
                        } else if let NetworkError::GetError(GetRecordError::NotEnoughCopies { .. }) = e {
                            debug!("Pad {} verified but not enough copies.", pad.address);
                            if let Ok(mut index_guard) = index.try_write() {
                                index_guard.verified_pending_pad(pad).unwrap();
                            } else {
                                error!("Could not acquire write lock for discarding pad {} (NotEnoughCopies)", pad.address);
                            }
                        } else if aggressive {
                            debug!("Pad {} discarded (aggressive).", pad.address);
                            if let Ok(mut index_guard) = index.try_write() {
                                index_guard.discard_pending_pad(pad).unwrap();
                            } else {
                                error!("Could not acquire write lock for discarding pad {} (aggressive)", pad.address);
                            }
                        } else {
                            warn!("Pad {} was found but got an error ({}). Leaving in pending until purge is run with aggressive flag", pad.address, e);
                        }
                        nb_failed_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }
                invoke_purge_callback(&task_callback, PurgeEvent::PadProcessed).await.unwrap();
            }));
        }

        while let Some(result) = tasks.next().await {
            match result {
                Ok(_) => { /* Task completed successfully (logic handled inside task) */ }
                Err(e) => {
                    error!("Purge task panicked: {:?}", e);
                }
            }
        }

        invoke_purge_callback(
            &callback,
            PurgeEvent::Complete {
                verified_count: nb_verified.load(Ordering::Relaxed),
                failed_count: nb_failed.load(Ordering::Relaxed),
            },
        )
        .await
        .unwrap();

        Ok(PurgeResult {
            nb_pads_purged: nb_failed.load(Ordering::Relaxed),
        })
    }

    pub async fn health_check(
        &self,
        key_name: &str,
        recycle: bool,
        health_check_callback: Option<HealthCheckCallback>,
    ) -> Result<HealthCheckResult, Error> {
        let pads = self.index.read().await.get_pads(key_name);
        let nb_recycled = Arc::new(AtomicUsize::new(0));
        let nb_reset = Arc::new(AtomicUsize::new(0));
        let callback = health_check_callback.clone();

        invoke_health_check_callback(
            &callback,
            HealthCheckEvent::Starting {
                total_keys: pads.len(),
            },
        )
        .await
        .unwrap();

        let is_public = self.index.read().await.is_public(key_name);
        let mut tasks = Vec::new();
        let client_guard = self
            .network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client = Arc::new(client_guard);

        for pad in pads {
            let pad = pad.clone();
            let nb_recycled_clone = nb_recycled.clone();
            let nb_reset_clone = nb_reset.clone();
            let key_name = key_name.to_string();
            let network = self.network.clone();
            let task_callback = callback.clone();
            let index = self.index.clone();
            let client_clone = client.clone();

            tasks.push(tokio::spawn(async move {
                let secret_key_owned;
                let secret_key_ref = if is_public {
                    None
                } else {
                    secret_key_owned = pad.secret_key();
                    Some(&secret_key_owned)
                };

                if pad.status != PadStatus::Confirmed {
                    return;
                }

                let client_ref = &*client_clone;
                match network.get(client_ref, &pad.address, secret_key_ref).await {
                    Ok(_) => {
                        invoke_health_check_callback(
                            &task_callback,
                            HealthCheckEvent::KeyProcessed,
                        )
                        .await
                        .unwrap();
                        return;
                    }
                    Err(e) => {
                        warn!(
                            "Error getting pad {} during health check: {}",
                            pad.address, e
                        );
                        match e {
                            NetworkError::GetError(GetRecordError::RecordNotFound)
                            | NetworkError::GetError(GetRecordError::NotEnoughCopies { .. }) => {
                                let mut index_guard = index.write().await;
                                index_guard
                                    .update_pad_status(
                                        &key_name,
                                        &pad.address,
                                        PadStatus::Free,
                                        None,
                                    )
                                    .unwrap();
                                nb_reset_clone.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {
                                if recycle {
                                    let mut index_guard = index.write().await;
                                    index_guard
                                        .recycle_errored_pad(&key_name, &pad.address)
                                        .unwrap();
                                }
                                nb_recycled_clone.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        invoke_health_check_callback(
                            &task_callback,
                            HealthCheckEvent::KeyProcessed,
                        )
                        .await
                        .unwrap();
                    }
                }
            }));
        }

        let results = futures::future::join_all(tasks).await;

        for result in results {
            match result {
                Ok(_) => {}
                Err(e) => {
                    error!("Health check task panicked: {:?}", e);
                }
            }
        }

        invoke_health_check_callback(
            &callback,
            HealthCheckEvent::Complete {
                nb_keys_updated: nb_reset.load(Ordering::Relaxed),
            },
        )
        .await
        .unwrap();

        println!(
            "Health check completed. {} pads reset.",
            nb_reset.load(Ordering::Relaxed)
        );

        if nb_recycled.load(Ordering::Relaxed) > 0 {
            if recycle {
                println!(
                    "{} pads got errored and have been recycled.",
                    nb_recycled.load(Ordering::Relaxed)
                );
            } else {
                println!(
                    "{} pads got errored and should be recycled.",
                    nb_recycled.load(Ordering::Relaxed)
                );
                println!("You can re-run the health-check command with the --recycle flag to recycle them.");
            }
        }

        if nb_reset.load(Ordering::Relaxed) > 0
            || (nb_recycled.load(Ordering::Relaxed) > 0 && recycle)
        {
            println!("Please re-run the same put command you used before to resume the upload of the missing pads to the network.");
        }

        Ok(HealthCheckResult {
            nb_keys_reset: nb_reset.load(Ordering::Relaxed),
            nb_keys_recycled: nb_recycled.load(Ordering::Relaxed),
        })
    }

    pub async fn sync(
        &self,
        force: bool,
        sync_callback: Option<SyncCallback>,
    ) -> Result<SyncResult, Error> {
        let mut sync_result = SyncResult {
            nb_keys_added: 0,
            nb_keys_updated: 0,
            nb_free_pads_added: 0,
            nb_pending_pads_added: 0,
        };
        let callback = sync_callback.clone();

        invoke_sync_callback(&callback, SyncEvent::FetchingRemoteIndex)
            .await
            .unwrap();

        let owner_secret_key = self.network.secret_key();
        let (owner_address, owner_secret_key) =
            derive_master_index_info(&owner_secret_key.to_hex())?;

        let client_guard_get = self
            .network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client_get = &*client_guard_get;

        let (remote_index, remote_index_counter) = match self
            .network
            .get(client_get, &owner_address, Some(&owner_secret_key))
            .await
        {
            Ok(get_result) => {
                let remote_index = if force {
                    MasterIndex::new(self.network.network_choice())
                } else {
                    serde_cbor::from_slice(&get_result.data).unwrap()
                };

                (remote_index, get_result.counter)
            }
            Err(_e) => (MasterIndex::new(self.network.network_choice()), 0),
        };
        drop(client_guard_get);

        invoke_sync_callback(&callback, SyncEvent::Merging)
            .await
            .unwrap();

        let mut local_index = self.index.read().await.clone();

        if !force {
            for (key, remote_entry) in remote_index.list() {
                let local_entry = local_index.get_entry(&key);
                if local_entry.is_none() {
                    local_index.add_entry(&key, remote_entry)?;
                    sync_result.nb_keys_added += 1;
                } else {
                    if local_index.update_entry(&key, remote_entry)? {
                        sync_result.nb_keys_updated += 1;
                    }
                }
            }

            let mut free_pads_to_add = Vec::new();
            let mut pending_pads_to_add = Vec::new();

            for pad in remote_index.export_raw_pads_private_key()? {
                if local_index.pad_exists(&pad.address) {
                    continue;
                }
                if pad.status == PadStatus::Generated {
                    pending_pads_to_add.push(pad);
                    sync_result.nb_pending_pads_added += 1;
                } else {
                    free_pads_to_add.push(pad);
                    sync_result.nb_free_pads_added += 1;
                }
            }

            local_index.import_raw_pads_private_key(free_pads_to_add)?;
            local_index.import_raw_pads_private_key(pending_pads_to_add)?;
        }

        let client_guard_put = self
            .network
            .get_client(Config::Put)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client_put = &*client_guard_put;

        invoke_sync_callback(&callback, SyncEvent::PushingRemoteIndex)
            .await
            .unwrap();

        let serialized_index = serde_cbor::to_vec(&local_index).unwrap();

        let pad_info = PadInfo {
            address: owner_address,
            status: PadStatus::Confirmed,
            chunk_index: 0,
            size: serialized_index.len(),
            last_known_counter: remote_index_counter + 1,
            sk_bytes: owner_secret_key.to_bytes().to_vec(),
            checksum: 0,
        };

        self.network
            .put(
                client_put,
                &pad_info,
                &serialized_index,
                DATA_ENCODING_MASTER_INDEX,
                false,
            )
            .await?;
        drop(client_guard_put);

        let client_guard_verify = self
            .network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
        let client_verify = &*client_guard_verify;

        invoke_sync_callback(&callback, SyncEvent::VerifyingRemoteIndex)
            .await
            .unwrap();

        let mut retries = 10;

        loop {
            match self
                .network
                .get(client_verify, &owner_address, Some(&owner_secret_key))
                .await
            {
                Ok(get_result) => {
                    if get_result.data != serialized_index {
                    } else if get_result.counter != remote_index_counter + 1 {
                    } else {
                        break Ok(());
                    }
                }
                Err(_e) => {}
            };

            if retries == 0 {
                break Err(Error::Network(
                    NetworkError::GetError(GetRecordError::RecordNotFound).into(),
                ));
            }

            retries -= 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }?;

        drop(client_guard_verify);

        invoke_sync_callback(&callback, SyncEvent::Complete)
            .await
            .unwrap();

        Ok(sync_result)
    }
}

fn derive_master_index_info(
    private_key_hex: &str,
) -> Result<(ScratchpadAddress, SecretKey), Error> {
    debug!("Deriving master index key and address...");
    let hex_to_decode = private_key_hex
        .strip_prefix("0x")
        .unwrap_or(private_key_hex);

    let input_key_bytes = hex::decode(hex_to_decode)
        .map_err(|e| Error::Config(format!("Failed to decode private key hex: {}", e)))?;

    let mut hasher = Sha256::new();
    hasher.update(&input_key_bytes);
    let hash_result = hasher.finalize();
    let key_array: [u8; 32] = hash_result.into();

    let derived_key = SecretKey::from_bytes(key_array)
        .map_err(|e| Error::Internal(format!("Failed to create SecretKey from HASH: {:?}", e)))?;
    let derived_public_key = derived_key.public_key();
    let address = ScratchpadAddress::new(derived_public_key);
    info!("Derived Master Index Address: {}", address);
    Ok((address, derived_key))
}
