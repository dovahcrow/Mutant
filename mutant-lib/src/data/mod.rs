use crate::{
    index::{master_index::MasterIndex, PadInfo, PadStatus},
    internal_events::{invoke_put_callback, PutCallback, PutEvent},
    network::{Network, NetworkError},
};
use ant_networking::GetRecordError;
use autonomi::ScratchpadAddress;
use futures::StreamExt;
use log::{debug, error, info, warn};
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

mod error;
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
}

impl Data {
    pub fn new(
        network: Arc<Network>,
        index: Arc<RwLock<MasterIndex>>,
        put_callback: Option<PutCallback>,
    ) -> Self {
        Self {
            network,
            index,
            put_callback,
        }
    }

    pub fn set_put_callback(&mut self, callback: PutCallback) {
        self.put_callback = Some(callback);
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

        let initial_written_count = pads
            .iter()
            .filter(|p| p.status == PadStatus::Written)
            .count();

        let initial_chunks_to_reserve = pads
            .iter()
            .filter(|p| p.status == PadStatus::Generated)
            .count();

        let process_future = self.process_pads(
            context.clone(),
            pad_tx.clone(),
            pad_rx,
            initial_written_count,
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
        initial_written_count: usize,
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
                initial_written_count,
                initial_confirmed_count,
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
                                    PutEvent::ChunkWritten,
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
                                        confirmed_counter.fetch_add(1, Ordering::SeqCst);
                                    invoke_put_callback(
                                        &mut context.put_callback,
                                        PutEvent::ChunkConfirmed,
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

    async fn fetch_pads_data(&self, pads: Vec<PadInfo>, public: bool) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();
        let mut tasks = Vec::new();

        for pad in pads {
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

    pub async fn get_public(&self, address: &ScratchpadAddress) -> Result<Vec<u8>, Error> {
        let index_pad_data = self.network.get(address, None).await?;

        match index_pad_data.data_encoding {
            DATA_ENCODING_PUBLIC_INDEX => {
                let index: Vec<PadInfo> =
                    serde_cbor::from_slice(&index_pad_data.data).map_err(|e| {
                        Error::Internal(format!("Failed to decode public index: {}", e))
                    })?;
                self.fetch_pads_data(index, true).await
            }
            DATA_ENCODING_PUBLIC_DATA => Ok(index_pad_data.data),
            _ => Err(Error::Internal(format!(
                "Unexpected data encoding {} found for public address {}",
                index_pad_data.data_encoding, address
            ))),
        }
    }

    pub async fn get(&self, name: &str) -> Result<Vec<u8>, Error> {
        let pads = self.index.read().await.get_pads(name);

        if pads.is_empty() {
            return Err(Error::Internal(format!("No pads found for key {}", name)));
        }

        let is_public = self.index.read().await.is_public(name);

        let pads = if is_public && pads.len() > 1 {
            pads[1..].to_vec()
        } else {
            pads
        };

        self.fetch_pads_data(pads, is_public).await
    }

    // pub async fn get_public(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, name: &[u8]) -> Result<(), Error> {}

    pub async fn purge(&self, aggressive: bool) -> Result<(), Error> {
        let pads = self.index.read().await.get_pending_pads();

        debug!("Purging {} pads.", pads.len());

        let mut tasks = futures::stream::FuturesOrdered::new();

        for pad in pads {
            debug!("Verifying pad {}", pad.address);

            let pad = pad.clone();
            let network = self.network.clone();
            let index = self.index.clone();

            tasks.push_back(tokio::spawn(async move {
                match network.get(&pad.address, None).await {
                    Ok(_res) => {
                        debug!("Pad {} verified.", pad.address);
                        if let Ok(mut index_guard) = index.try_write() {
                            index_guard.verified_pending_pad(pad).unwrap();
                        } else {
                            error!("Could not acquire write lock for verifying pad {}", pad.address);
                        }
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
                    }
                }
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
        Ok(())
    }
}
