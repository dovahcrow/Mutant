use crate::{
    index::{master_index::MasterIndex, PadInfo, PadStatus},
    network::Network,
    storage::ScratchpadAddress,
};
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

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const CHUNK_PROCESSING_QUEUE_SIZE: usize = 256;
pub const PAD_RECYCLING_RETRIES: usize = 25;

mod error;
pub mod storage_mode;

pub use crate::internal_error::Error;

#[derive(Clone)]
struct Context {
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    name: Arc<String>,
    chunks: Vec<Vec<u8>>,
}

#[derive(Clone)]
pub struct Data {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
}

impl Data {
    pub fn new(network: Arc<Network>, index: Arc<RwLock<MasterIndex>>) -> Self {
        Self { network, index }
    }

    pub async fn put(&self, name: &str, data_bytes: &[u8], mode: StorageMode) -> Result<(), Error> {
        if self.index.read().await.contains_key(&name) {
            if self
                .index
                .read()
                .await
                .verify_checksum(&name, data_bytes, mode)
            {
                info!("Resume for {}", name);
                self.resume(name, data_bytes, mode).await
            } else {
                info!("Update for {}", name);
                self.index.write().await.remove_key(&name).unwrap();
                return self.first_store(name, data_bytes, mode).await;
            }
        } else {
            info!("First store for {}", name);
            self.first_store(name, data_bytes, mode).await
        }
    }

    async fn resume(&self, name: &str, data_bytes: &[u8], mode: StorageMode) -> Result<(), Error> {
        let pads = self.index.read().await.get_pads(name);

        // check pads are in the correct mode or remove key and start over
        if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
            self.index.write().await.remove_key(name).unwrap();
            return self.first_store(name, data_bytes, mode).await;
        }

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: data_bytes
                .chunks(mode.scratchpad_size())
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<_>>(),
        };

        self.write_pipeline(context, pads).await;

        Ok(())
    }

    async fn first_store(
        &self,
        name: &str,
        data_bytes: &[u8],
        mode: StorageMode,
    ) -> Result<(), Error> {
        let pads = self
            .index
            .write()
            .await
            .create_private_key(&name, data_bytes, mode)?;

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: data_bytes
                .chunks(mode.scratchpad_size())
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<_>>(),
        };

        self.write_pipeline(context, pads).await;

        Ok(())
    }

    async fn write_pipeline(&self, context: Context, pads: Vec<PadInfo>) {
        let (pad_tx, pad_rx) = channel(CHUNK_PROCESSING_QUEUE_SIZE);

        let initial_confirmed_count = pads
            .iter()
            .filter(|p| p.status == PadStatus::Confirmed)
            .count();

        let process_future = self.process_pads(
            context.clone(),
            pad_tx.clone(),
            pad_rx,
            initial_confirmed_count,
        );

        for pad in pads {
            let _ = pad_tx.send(pad.clone()).await;
        }

        drop(pad_tx);

        process_future.await.unwrap();
    }

    async fn process_pads(
        &self,
        context: Context,
        pad_tx: Sender<PadInfo>,
        mut pad_rx: Receiver<PadInfo>,
        initial_confirmed_count: usize,
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
                                    pad,
                                    task_pad_tx,
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

        Ok(())
    }

    async fn process_pad_task(
        context: Context,
        confirmed_counter: Arc<AtomicUsize>,
        pad: PadInfo,
        pad_tx: Sender<PadInfo>,
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

        if initial_status == PadStatus::Generated || initial_status == PadStatus::Free {
            let mut retries_left = PAD_RECYCLING_RETRIES;
            loop {
                let put_result = context
                    .network
                    .put_private(
                        &pad,
                        &context.chunks[pad.chunk_index],
                        DATA_ENCODING_PRIVATE_DATA,
                    )
                    .await;

                match put_result {
                    Ok(_) => {
                        match context.index.write().await.update_pad_status(
                            key_name,
                            &current_pad_address,
                            PadStatus::Written,
                        ) {
                            Ok(updated_pad) => {
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
                            current_pad_address, pad.chunk_index, e, retries_left
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
            let mut retries_left = PAD_RECYCLING_RETRIES;
            loop {
                let get_result = context.network.get_private(&pad_to_confirm).await;

                if let Ok(gotten_pad) = get_result {
                    if (pad_to_confirm.last_known_counter == 0 && gotten_pad.counter == 0)
                        || pad_to_confirm.last_known_counter <= gotten_pad.counter
                    {
                        match context.index.write().await.update_pad_status(
                            key_name,
                            &current_pad_address,
                            PadStatus::Confirmed,
                        ) {
                            Ok(_) => {
                                let previous_count =
                                    confirmed_counter.fetch_add(1, Ordering::SeqCst);
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
                    }
                }

                retries_left -= 1;
                if retries_left == 0 {
                    warn!(
                        "Failed to confirm pad {} (chunk {}) after multiple retries. Recycling.",
                        current_pad_address, pad.chunk_index
                    );
                    recycle_pad().await;
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
    }

    pub async fn get(&self, name: &str) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();

        let pads = self.index.read().await.get_pads(name);

        let mut tasks = Vec::new();
        for pad in pads {
            let network = self.network.clone();
            tasks.push(tokio::spawn(async move {
                let mut retries_left = PAD_RECYCLING_RETRIES;
                let pad = loop {
                    match network.get_private(&pad).await {
                        Ok(pad) => break pad,
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

                Ok(pad.data)
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

    // pub async fn get_public(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, name: &[u8]) -> Result<(), Error> {}

    pub async fn purge(&self) -> Result<(), Error> {
        let pads = self.index.read().await.get_pending_pads();

        debug!("Purging {} pads.", pads.len());

        println!("Index: {:#?}", self.index.read().await);

        let mut tasks = Vec::new();

        for pad in pads {
            debug!("Verifying pad {}", pad.address);

            let pad = pad.clone();
            let network = self.network.clone();
            let index = self.index.clone();

            tasks.push(tokio::spawn(async move {
                match network.get_private(&pad).await {
                    Ok(_res) => {
                        debug!("Pad {} verified.", pad.address);
                        index.write().await.verified_pending_pad(pad).unwrap();
                    }
                    Err(e) => {
                        debug!("Pad {} discarded.", pad.address);
                        index.write().await.discard_pending_pad(pad).unwrap();
                    }
                }
            }));
        }

        let results = futures::future::join_all(tasks).await;
        for result in results {
            result.unwrap();
        }
        Ok(())
    }
}
