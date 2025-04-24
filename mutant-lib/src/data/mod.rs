use crate::{
    events::{GetCallback, GetEvent, PurgeCallback, PurgeEvent, SyncCallback, SyncEvent},
    index::{master_index::MasterIndex, PadInfo, PadStatus},
    internal_events::{
        invoke_get_callback, invoke_purge_callback, invoke_put_callback, invoke_sync_callback,
        PutCallback, PutEvent,
    },
    network::{Network, NetworkError},
};
use ant_networking::GetRecordError;
use autonomi::ScratchpadAddress;
use blsttc::SecretKey;
use futures::StreamExt;
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use storage_mode::StorageMode;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::RwLock;
use tokio::time::Instant;

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const CHUNK_PROCESSING_QUEUE_SIZE: usize = 256;
pub const PAD_RECYCLING_RETRIES: usize = 3;

const MAX_CONFIRMATION_DURATION: Duration = Duration::from_secs(60 * 10);

pub mod storage_mode;

pub use crate::internal_error::Error;

#[derive(Clone)]
struct Context {
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    name: Arc<String>,
    chunks: Vec<Vec<u8>>,
    put_callback: Option<PutCallback>,
}

pub struct Data {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
    put_callback: Option<PutCallback>,
    get_callback: Option<GetCallback>,
    purge_callback: Option<PurgeCallback>,
    sync_callback: Option<SyncCallback>,
}

impl Data {
    pub fn new(
        network: Arc<Network>,
        index: Arc<RwLock<MasterIndex>>,
        put_callback: Option<PutCallback>,
        get_callback: Option<GetCallback>,
        purge_callback: Option<PurgeCallback>,
        sync_callback: Option<SyncCallback>,
    ) -> Self {
        Self {
            network,
            index,
            put_callback,
            get_callback,
            purge_callback,
            sync_callback,
        }
    }

    pub fn set_put_callback(&mut self, callback: PutCallback) {
        self.put_callback = Some(callback);
    }

    pub fn set_get_callback(&mut self, callback: GetCallback) {
        self.get_callback = Some(callback);
    }

    pub fn set_purge_callback(&mut self, callback: PurgeCallback) {
        self.purge_callback = Some(callback);
    }

    pub fn set_sync_callback(&mut self, callback: SyncCallback) {
        self.sync_callback = Some(callback);
    }

    pub async fn put(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
    ) -> Result<ScratchpadAddress, Error> {
        if self.index.read().await.contains_key(&name) {
            if self
                .index
                .read()
                .await
                .verify_checksum(&name, data_bytes, mode)
            {
                info!("Resume for {}", name);
                self.resume(name, data_bytes, mode, public).await
            } else {
                info!("Update for {}", name);
                self.index.write().await.remove_key(&name).unwrap();
                return self.first_store(name, data_bytes, mode, public).await;
            }
        } else {
            info!("First store for {}", name);
            self.first_store(name, data_bytes, mode, public).await
        }
    }

    async fn resume(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
    ) -> Result<ScratchpadAddress, Error> {
        let pads = self.index.read().await.get_pads(name);

        // check pads are in the correct mode or remove key and start over
        if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
            self.index.write().await.remove_key(name).unwrap();
            return self.first_store(name, data_bytes, mode, public).await;
        }

        let chunks = self.index.read().await.chunk_data(data_bytes, mode, public);

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks,
            put_callback: self.put_callback.clone(),
        };

        self.write_pipeline(context, pads.clone(), public).await;

        Ok(pads[0].address)
    }

    async fn first_store(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
        public: bool,
    ) -> Result<ScratchpadAddress, Error> {
        let (pads, chunks) = self
            .index
            .write()
            .await
            .create_key(name, data_bytes, mode, public)?;

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks,
            put_callback: self.put_callback.clone(),
        };

        self.write_pipeline(context, pads.clone(), public).await;

        Ok(pads[0].address)
    }

    async fn write_pipeline(&self, context: Context, pads: Vec<PadInfo>, public: bool) {
        let (pad_tx, pad_rx) = channel(CHUNK_PROCESSING_QUEUE_SIZE);

        let initial_confirmed_count = pads
            .iter()
            .filter(|p| p.status == PadStatus::Confirmed)
            .count();

        let initial_chunks_to_reserve = pads
            .iter()
            .filter(|p| p.status == PadStatus::Generated)
            .count();

        let process_future = self.process_pads(
            context.clone(),
            pad_tx.clone(),
            pad_rx,
            initial_confirmed_count,
            initial_chunks_to_reserve,
            public,
        );

        for pad in pads {
            let _ = pad_tx.send(pad.clone()).await;
        }

        drop(pad_tx);

        process_future.await.unwrap();
    }

    async fn process_pads(
        &self,
        mut context: Context,
        pad_tx: Sender<PadInfo>,
        mut pad_rx: Receiver<PadInfo>,
        initial_confirmed_count: usize,
        initial_chunks_to_reserve: usize,
        public: bool,
    ) -> Result<(), tokio::task::JoinError> {
        let key_name = context.name.clone();
        let total_pads = context.chunks.len();
        let confirmed_pads = Arc::new(AtomicUsize::new(initial_confirmed_count));

        if initial_confirmed_count == total_pads {
            info!(
                "All {} pads for key {} were already confirmed. Skipping pipeline.",
                total_pads, key_name
            );
            return Ok(());
        }

        let mut tasks = futures::stream::FuturesUnordered::new();
        let mut outstanding_tasks = 0u32;
        let mut channel_closed = false;
        info!(
            "Starting pad processing pipeline for {} with {} pads ({} initially confirmed)",
            key_name, total_pads, initial_confirmed_count
        );

        invoke_put_callback(
            &mut context.put_callback,
            PutEvent::Starting {
                total_chunks: total_pads,
                initial_written_count: 0,
                initial_confirmed_count: 0,
                chunks_to_reserve: initial_chunks_to_reserve,
            },
        )
        .await
        .unwrap();

        loop {
            tokio::select! {
                res = pad_rx.recv(), if !channel_closed => {
                    match res {
                        Some(pad) => {
                            outstanding_tasks += 1;
                            let task_context = context.clone();
                            let task_confirmed_counter = confirmed_pads.clone();
                            let task_pad_tx = pad_tx.clone();
                            tasks.push(tokio::spawn(async move {
                                Self::process_pad_task(
                                    task_context,
                                    task_confirmed_counter,
                                    pad.clone(),
                                    task_pad_tx,
                                    public,
                                    if public && pad.chunk_index == 0 && total_pads > 1 {
                                        true
                                    } else {
                                        false
                                    },
                                ).await;
                            }));
                        },
                        None => {
                            channel_closed = true;
                            if outstanding_tasks == 0 {
                                break;
                            }
                        }
                    }
                },

                Some(result) = tasks.next(), if outstanding_tasks > 0 => {
                    outstanding_tasks -= 1;
                    if let Err(e) = result {
                         error!("process_pads: Sub-task panicked: {:?}", e);
                    }

                    if confirmed_pads.load(Ordering::SeqCst) == total_pads {
                        info!("All {} pads confirmed for key {}", total_pads, key_name);
                        break;
                    }

                    if channel_closed && outstanding_tasks == 0 {
                         warn!("Pad processing channel closed but not all pads confirmed ({} / {}) for key {}. Exiting loop.", confirmed_pads.load(Ordering::SeqCst), total_pads, key_name);
                         break;
                    }
                },

                else => {
                     info!("Pad processing loop exiting for key {}", key_name);
                     break;
                }
            }
        }
        info!("Finished pad processing pipeline for {}", key_name);

        invoke_put_callback(&mut context.put_callback, PutEvent::Complete)
            .await
            .unwrap();

        Ok(())
    }

    async fn process_pad_task(
        mut context: Context,
        confirmed_counter: Arc<AtomicUsize>,
        pad: PadInfo,
        pad_tx: Sender<PadInfo>,
        public: bool,
        is_index: bool,
    ) {
        let current_pad_address = pad.address;
        let key_name = &context.name;
        let mut pad_for_confirm: Option<PadInfo> = None;

        let initial_status = pad.status;

        if initial_status == PadStatus::Confirmed {
            debug!(
                "Pad {} already confirmed, skipping processing.",
                current_pad_address
            );

            confirmed_counter.fetch_add(1, Ordering::Relaxed);

            invoke_put_callback(&mut context.put_callback, PutEvent::PadsWritten)
                .await
                .unwrap();
            invoke_put_callback(&mut context.put_callback, PutEvent::PadsConfirmed)
                .await
                .unwrap();

            return;
        }

        let recycle_pad = async || match context
            .index
            .write()
            .await
            .recycle_errored_pad(key_name, &current_pad_address)
        {
            Ok(new_pad) => {
                if let Err(e) = pad_tx.send(new_pad).await {
                    error!(
                        "Failed to send recycled pad back to queue for key {}: {}",
                        key_name, e
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to recycle pad {} for key {}: {}",
                    current_pad_address, key_name, e
                );
            }
        };

        let encoding = if is_index {
            DATA_ENCODING_PUBLIC_INDEX
        } else {
            if public {
                DATA_ENCODING_PUBLIC_DATA
            } else {
                DATA_ENCODING_PRIVATE_DATA
            }
        };

        if initial_status == PadStatus::Generated || initial_status == PadStatus::Free {
            let mut retries_left = PAD_RECYCLING_RETRIES;
            loop {
                let put_result = context
                    .network
                    .put(&pad, &context.chunks[pad.chunk_index], encoding, public)
                    .await;

                match put_result {
                    Ok(_) => {
                        match context.index.write().await.update_pad_status(
                            key_name,
                            &current_pad_address,
                            PadStatus::Written,
                            None,
                        ) {
                            Ok(updated_pad) => {
                                invoke_put_callback(
                                    &mut context.put_callback,
                                    PutEvent::PadsWritten,
                                )
                                .await
                                .unwrap();

                                if initial_status == PadStatus::Generated {
                                    invoke_put_callback(
                                        &mut context.put_callback,
                                        PutEvent::PadReserved,
                                    )
                                    .await
                                    .unwrap();
                                }

                                pad_for_confirm = Some(updated_pad);

                                break;
                            }
                            Err(e) => {
                                unimplemented!("Failed to update pad status to Written: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Error putting pad {} (chunk {}): {}. Retries left: {}",
                            current_pad_address,
                            pad.chunk_index,
                            e,
                            retries_left - 1
                        );
                        retries_left -= 1;
                        if retries_left == 0 {
                            warn!(
                                "Failed to put pad {} (chunk {}) after multiple retries. Recycling.",
                                current_pad_address, pad.chunk_index
                            );
                            recycle_pad().await;
                            return;
                        }
                        continue;
                    }
                }
            }
        } else if initial_status == PadStatus::Written {
            pad_for_confirm = Some(pad.clone());

            invoke_put_callback(&mut context.put_callback, PutEvent::PadsWritten)
                .await
                .unwrap();
        }

        if let Some(pad_to_confirm) = pad_for_confirm {
            let start_time = Instant::now();

            loop {
                if start_time.elapsed() >= MAX_CONFIRMATION_DURATION {
                    warn!(
                        "Failed to confirm pad {} (chunk {}) within time budget ({:?}). Recycling.",
                        current_pad_address, pad.chunk_index, MAX_CONFIRMATION_DURATION
                    );
                    recycle_pad().await;
                    return;
                }

                let secret_key = pad_to_confirm.secret_key();

                let secret_key = if public { None } else { Some(&secret_key) };

                let get_result = context
                    .network
                    .get(&pad_to_confirm.address, secret_key)
                    .await;

                match get_result {
                    Ok(gotten_pad) => {
                        if (pad_to_confirm.last_known_counter == 0 && gotten_pad.counter == 0)
                            || pad_to_confirm.last_known_counter <= gotten_pad.counter
                        {
                            match context.index.write().await.update_pad_status(
                                key_name,
                                &current_pad_address,
                                PadStatus::Confirmed,
                                Some(gotten_pad.counter),
                            ) {
                                Ok(_) => {
                                    let previous_count =
                                        confirmed_counter.fetch_add(1, Ordering::Relaxed);
                                    invoke_put_callback(
                                        &mut context.put_callback,
                                        PutEvent::PadsConfirmed,
                                    )
                                    .await
                                    .unwrap();
                                    debug!(
                                        "Pad {} confirmed. Confirmed count: {}",
                                        current_pad_address,
                                        previous_count + 1
                                    );
                                }
                                Err(e) => error!(
                                    "Failed to update pad {} status to Confirmed: {}",
                                    current_pad_address, e
                                ),
                            };
                            return;
                        } else {
                            // Counter condition not met, log and retry immediately
                            debug!(
                                "Pad {} counter check failed (last_known: {}, gotten: {}). Retrying.",
                                current_pad_address, pad_to_confirm.last_known_counter, gotten_pad.counter
                             );
                        }
                    }
                    Err(e) => {
                        // get_private failed, log and retry immediately
                        debug!(
                            "Error getting pad {} for confirmation: {}. Retrying.",
                            current_pad_address, e
                        );
                    }
                }

                // No sleep here, loop immediately to retry if not confirmed or error occurred
                continue;
            }
        }
    }

    async fn fetch_pads_data(
        &mut self,
        pads: Vec<PadInfo>,
        public: bool,
    ) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();
        let mut tasks = Vec::new();

        for pad in pads {
            let mut get_callback = self.get_callback.clone();
            let network = self.network.clone();
            tasks.push(tokio::spawn(async move {
                let mut retries_left = PAD_RECYCLING_RETRIES;
                let owned_key;
                let secret_key_ref = if public {
                    None
                } else {
                    owned_key = pad.secret_key();
                    Some(&owned_key)
                };

                let pad_data = loop {
                    match network.get(&pad.address, secret_key_ref).await {
                        Ok(pad_result) => {
                            if pad_result.data.len() != pad.size {
                                return Err(Error::Internal(format!(
                                    "Pad size mismatch for pad {}: expected {}, got {}",
                                    pad.address,
                                    pad.size,
                                    pad_result.data.len()
                                )));
                            }

                            invoke_get_callback(&mut get_callback, GetEvent::PadsFetched)
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

        invoke_get_callback(&mut self.get_callback, GetEvent::Complete)
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

    pub async fn get_public(&mut self, address: &ScratchpadAddress) -> Result<Vec<u8>, Error> {
        let index_pad_data = self.network.get(address, None).await?;

        match index_pad_data.data_encoding {
            DATA_ENCODING_PUBLIC_INDEX => {
                let index: Vec<PadInfo> =
                    serde_cbor::from_slice(&index_pad_data.data).map_err(|e| {
                        Error::Internal(format!("Failed to decode public index: {}", e))
                    })?;

                invoke_get_callback(&mut self.get_callback, GetEvent::PadsFetched)
                    .await
                    .unwrap();

                self.fetch_pads_data(index, true).await
            }
            DATA_ENCODING_PUBLIC_DATA => Ok(index_pad_data.data),
            _ => Err(Error::Internal(format!(
                "Unexpected data encoding {} found for public address {}",
                index_pad_data.data_encoding, address
            ))),
        }
    }

    pub async fn get(&mut self, name: &str) -> Result<Vec<u8>, Error> {
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

        invoke_get_callback(
            &mut self.get_callback,
            GetEvent::Starting {
                total_chunks: pads.len(),
            },
        )
        .await
        .unwrap();

        let is_public = self.index.read().await.is_public(name);

        let pads = if is_public && pads.len() > 1 {
            pads[1..].to_vec()
        } else {
            pads
        };

        self.fetch_pads_data(pads, is_public).await
    }

    pub async fn purge(&mut self, aggressive: bool) -> Result<(), Error> {
        let pads = self.index.read().await.get_pending_pads();

        let nb_verified = Arc::new(AtomicUsize::new(0));
        let nb_failed = Arc::new(AtomicUsize::new(0));

        debug!("Purging {} pads.", pads.len());

        invoke_purge_callback(
            &mut self.purge_callback,
            PurgeEvent::Starting {
                total_count: pads.len(),
            },
        )
        .await
        .unwrap();

        let mut tasks = futures::stream::FuturesOrdered::new();

        for pad in pads {
            debug!("Verifying pad {}", pad.address);

            let pad = pad.clone();
            let network = self.network.clone();
            let index = self.index.clone();
            let nb_verified = nb_verified.clone();
            let nb_failed = nb_failed.clone();
            let mut purge_callback = self.purge_callback.clone();

            tasks.push_back(tokio::spawn(async move {
                match network.get(&pad.address, None).await {
                    Ok(_res) => {
                        debug!("Pad {} verified.", pad.address);
                        if let Ok(mut index_guard) = index.try_write() {
                            index_guard.verified_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for verifying pad {}", pad.address);
                        }

                        nb_verified.fetch_add(1, Ordering::Relaxed);
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

                        nb_failed.fetch_add(1, Ordering::Relaxed);
                    }
                }

                invoke_purge_callback(&mut purge_callback, PurgeEvent::PadProcessed)
                    .await
                    .unwrap();
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
            &mut self.purge_callback,
            PurgeEvent::Complete {
                verified_count: nb_verified.load(Ordering::Relaxed),
                failed_count: nb_failed.load(Ordering::Relaxed),
            },
        )
        .await
        .unwrap();

        Ok(())
    }

    pub async fn health_check(&mut self, key_name: &str, recycle: bool) -> Result<(), Error> {
        let pads = self.index.read().await.get_pads(key_name);
        let nb_recycled = Arc::new(AtomicUsize::new(0));
        let nb_reset = Arc::new(AtomicUsize::new(0));
        invoke_get_callback(
            &mut self.get_callback,
            GetEvent::Starting {
                total_chunks: pads.len(),
            },
        )
        .await
        .unwrap();

        let is_public = self.index.read().await.is_public(key_name);

        let mut tasks = Vec::new();

        for pad in pads {
            let pad = pad.clone();
            let nb_recycled = nb_recycled.clone();
            let nb_reset = nb_reset.clone();
            let key_name = key_name.to_string();
            let network = self.network.clone();
            let mut get_callback = self.get_callback.clone();
            let index = self.index.clone();
            let secret_key = if is_public {
                None
            } else {
                Some(pad.secret_key())
            };

            tasks.push(tokio::spawn(async move {
                if pad.status != PadStatus::Confirmed {
                    return;
                }

                match network.get(&pad.address, secret_key.as_ref()).await {
                    Ok(_) => {
                        invoke_get_callback(&mut get_callback, GetEvent::PadsFetched)
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

                                nb_reset.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {
                                if recycle {
                                    let mut index_guard = index.write().await;
                                    index_guard
                                        .recycle_errored_pad(&key_name, &pad.address)
                                        .unwrap();
                                }

                                nb_recycled.fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        invoke_get_callback(&mut get_callback, GetEvent::PadsFetched)
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

        invoke_get_callback(&mut self.get_callback, GetEvent::Complete)
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

        Ok(())
    }

    /// Get the master index from the network and merge it into a copy of the local index.
    /// The merge is additive, it will only add or synchronize existing keys and not delete any.
    /// The merge also synchronize the free and pending pads.
    /// We take care when adding or synchronizing pads that it does not already exist anywhere.
    /// If a key exists in the local and the remote, we take the latest version (check the pads counters)
    pub async fn sync(&mut self, force: bool) -> Result<SyncResult, Error> {
        let mut sync_result = SyncResult {
            nb_keys_added: 0,
            nb_keys_updated: 0,
            nb_free_pads_added: 0,
            nb_pending_pads_added: 0,
        };

        invoke_sync_callback(&mut self.sync_callback, SyncEvent::FetchingRemoteIndex)
            .await
            .unwrap();

        // we need to hash the secret key to avoid overriding the vault that exists at the original secret key location
        let owner_secret_key = self.network.secret_key();
        let (owner_address, owner_secret_key) =
            derive_master_index_info(&owner_secret_key.to_hex())?;

        let (remote_index, remote_index_counter) = match self
            .network
            .get(&owner_address, Some(&owner_secret_key))
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

        invoke_sync_callback(&mut self.sync_callback, SyncEvent::Merging)
            .await
            .unwrap();

        let mut local_index = self.index.read().await.clone();

        if !force {
            // Merge the remote index into the local index
            for (key, remote_entry) in remote_index.list() {
                let local_entry = local_index.get_entry(&key);
                if local_entry.is_none() {
                    // Key does not exist in local index, add it
                    local_index.add_entry(&key, remote_entry)?;
                    sync_result.nb_keys_added += 1;
                } else {
                    // Key exists in local index, update it if the remote entry is newer
                    if local_index.update_entry(&key, remote_entry)? {
                        sync_result.nb_keys_updated += 1;
                    }
                }
            }

            let mut free_pads_to_add = Vec::new();
            let mut pending_pads_to_add = Vec::new();

            // merge the free pads
            // check if the pad exists anywhere in the local index
            for pad in remote_index.export_raw_pads_private_key()? {
                if local_index.pad_exists(&pad.address) {
                    continue;
                }
                free_pads_to_add.push(pad);
                sync_result.nb_free_pads_added += 1;
            }

            // merge the pending pads
            for pad in remote_index.get_pending_pads() {
                if local_index.pad_exists(&pad.address) {
                    continue;
                }
                pending_pads_to_add.push(pad);
                sync_result.nb_pending_pads_added += 1;
            }

            local_index.import_raw_pads_private_key(free_pads_to_add)?;
            local_index.import_raw_pads_private_key(pending_pads_to_add)?;
        }

        invoke_sync_callback(&mut self.sync_callback, SyncEvent::PushingRemoteIndex)
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

        // push the remote index to the network
        self.network
            .put(
                &pad_info,
                &serialized_index,
                DATA_ENCODING_MASTER_INDEX,
                false,
            )
            .await?;

        invoke_sync_callback(&mut self.sync_callback, SyncEvent::VerifyingRemoteIndex)
            .await
            .unwrap();

        let mut retries = 10;

        loop {
            match self
                .network
                .get(&owner_address, Some(&owner_secret_key))
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
        }?;

        invoke_sync_callback(&mut self.sync_callback, SyncEvent::Complete)
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

pub struct SyncResult {
    pub nb_keys_added: usize,
    pub nb_keys_updated: usize,
    pub nb_free_pads_added: usize,
    pub nb_pending_pads_added: usize,
}
