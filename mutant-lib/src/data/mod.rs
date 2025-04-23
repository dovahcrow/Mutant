use crate::{
    index::{master_index::MasterIndex, PadInfo, PadStatus, DEFAULT_SCRATCHPAD_SIZE},
    network::Network,
    storage::ScratchpadAddress,
};
use log::{debug, error, info, warn};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::RwLock;

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const CHUNK_PROCESSING_QUEUE_SIZE: usize = 256;

mod error;

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

    pub async fn put(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
        if self.index.read().await.contains_key(&name) {
            if self.index.read().await.verify_checksum(&name, data_bytes) {
                info!("Resume for {}", name);
                self.resume(name, data_bytes).await
            } else {
                info!("Update for {}", name);
                self.index.write().await.remove_key(&name).unwrap();
                return self.first_store(name, data_bytes).await;
            }
        } else {
            info!("First store for {}", name);
            self.first_store(name, data_bytes).await
        }
    }

    async fn resume(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
        let pads = self.index.read().await.get_pads(name);

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: data_bytes
                .chunks(DEFAULT_SCRATCHPAD_SIZE)
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<_>>(),
        };

        self.write_pipeline(context, pads).await;

        Ok(())
    }

    async fn first_store(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
        let pads = self
            .index
            .write()
            .await
            .create_private_key(&name, data_bytes)?;

        let context = Context {
            index: self.index.clone(),
            network: self.network.clone(),
            name: Arc::new(name.to_string()),
            chunks: data_bytes
                .chunks(DEFAULT_SCRATCHPAD_SIZE)
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<_>>(),
        };

        self.write_pipeline(context, pads).await;

        Ok(())
    }

    async fn write_pipeline(&self, context: Context, pads: Vec<PadInfo>) {
        let (pad_tx, pad_rx) = channel(CHUNK_PROCESSING_QUEUE_SIZE);
        let key = context.name.clone();
        debug!(
            "write_pipeline: Starting single-loop pipeline for key {}",
            key
        );

        let process_future = self.process_pads(context.clone(), pad_tx.clone(), pad_rx);

        debug!(
            "write_pipeline: Sending {} initial pads for key {}",
            pads.len(),
            key
        );
        for pad in pads {
            if pad_tx.send(pad.clone()).await.is_err() {
                error!(
                    "write_pipeline: failed to send initial pad {} to processing channel; receiver closed prematurely.",
                    pad.address
                );
            }
        }

        debug!(
            "write_pipeline: Dropping initial pad sender for key {}",
            key
        );
        drop(pad_tx);

        debug!(
            "write_pipeline: Waiting for processing task for key {}",
            key
        );
        if let Err(e) = process_future.await {
            error!(
                "write_pipeline: Processing task for key {} panicked: {:?}",
                key, e
            );
        }
        debug!("write_pipeline: Completed pipeline for key {}", key);
    }

    async fn process_pads(
        &self,
        context: Context,
        pad_tx: Sender<PadInfo>,
        mut pad_rx: Receiver<PadInfo>,
    ) -> Result<(), tokio::task::JoinError> {
        let key_name = context.name.clone();
        debug!(
            "process_pads: Starting main receiver loop for key {}",
            key_name
        );
        let mut tasks = Vec::new();

        while let Some(pad) = pad_rx.recv().await {
            debug!("process_pads: Received pad {} for processing.", pad.address);
            let task_context = context.clone();
            let task_pad_tx = pad_tx.clone();
            tasks.push(tokio::spawn(async move {
                Self::process_pad_task(task_context, task_pad_tx, pad).await;
            }));
        }

        debug!("process_pads: Main receiver loop finished for key {}. Waiting for {} outstanding tasks.", key_name, tasks.len());
        drop(pad_tx);

        let results = futures::future::join_all(tasks).await;
        debug!(
            "process_pads: Finished joining sub-tasks for key {}.",
            key_name
        );
        for result in results {
            if let Err(e) = result {
                error!("process_pads: Sub-task panicked: {:?}", e);
            }
        }
        debug!("process_pads: All sub-tasks joined for key {}.", key_name);
        Ok(())
    }

    async fn process_pad_task(context: Context, pad_tx: Sender<PadInfo>, pad: PadInfo) {
        let current_pad_address = pad.address;
        let key_name = &context.name;
        let mut pad_for_confirm: Option<PadInfo> = None;
        debug!(
            "process_pad_task: Started processing pad {} for key {}",
            current_pad_address, key_name
        );

        let recycle_and_resend = |index: Arc<RwLock<MasterIndex>>,
                                  pad_tx: Sender<PadInfo>,
                                  reason: String,
                                  key_name: String,
                                  address: ScratchpadAddress| async move {
            warn!(
                "Recycling pad {} for key {} because of {}",
                address, key_name, reason
            );
            match index.write().await.recycle_errored_pad(&key_name, &address) {
                Ok(new_pad) => {
                    debug!(
                        "Sending recycled pad {} back to processing queue for key {}.",
                        new_pad.address, key_name
                    );
                    if pad_tx.send(new_pad).await.is_err() {
                        error!(
                            "Failed to send recycled pad {} for key {} to channel; receiver closed.",
                             address, key_name
                        );
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to recycle pad {} for key {}: {:?}",
                        address, key_name, e
                    );
                }
            }
        };

        let initial_status = pad.status;

        if initial_status == PadStatus::Confirmed {
            debug!(
                "Skipping pad {} (key {}) as it's already confirmed.",
                current_pad_address, key_name
            );
            return;
        }

        let mut status_after_put = initial_status;
        if initial_status == PadStatus::Generated || initial_status == PadStatus::Free {
            debug!(
                "Attempting PUT for pad {} (key {}, status {:?})",
                current_pad_address, key_name, initial_status
            );
            let mut retries_left = 3;
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
                        debug!(
                            "PUT successful for pad {} (key {}). Updating status to Written.",
                            current_pad_address, key_name
                        );
                        match context.index.write().await.update_pad_status(
                            key_name,
                            &current_pad_address,
                            PadStatus::Written,
                        ) {
                            Ok(updated_pad) => {
                                status_after_put = updated_pad.status;
                                pad_for_confirm = Some(updated_pad);
                                break;
                            }
                            Err(e) => {
                                error!(
                                    "Failed to update status to Written for pad {} (key {}): {:?}. Aborting task.",
                                    current_pad_address, key_name, e
                                );
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        debug!(
                            "Error putting pad {} (key {}): {}",
                            current_pad_address, key_name, e
                        );
                        retries_left -= 1;
                        if retries_left == 0 {
                            recycle_and_resend(
                                context.index.clone(),
                                pad_tx.clone(),
                                "persistent PUT error".to_string(),
                                key_name.to_string(),
                                current_pad_address,
                            )
                            .await;
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                }
            }
        }

        if let Some(pad_to_confirm) = pad_for_confirm {
            if pad_to_confirm.status != PadStatus::Written {
                debug!(
                    "Pad {} (key {}) status changed to {:?} before confirmation check could run. Skipping confirm.",
                    current_pad_address, key_name, pad_to_confirm.status
                 );
                return;
            }

            debug!(
                "Attempting GET/Confirm for pad {} (key {})",
                current_pad_address, key_name
            );
            let mut retries_left = 3;
            loop {
                let get_result = context.network.get_private(&pad_to_confirm).await;

                match get_result {
                    Ok(gotten_pad) => {
                        if (pad_to_confirm.last_known_counter == 0 && gotten_pad.counter == 0)
                            || pad_to_confirm.last_known_counter <= gotten_pad.counter
                        {
                            debug!(
                                "GET/Confirm successful for pad {} (key {}). Updating status to Confirmed.",
                                current_pad_address, key_name
                            );
                            match context.index.write().await.update_pad_status(
                                key_name,
                                &current_pad_address,
                                PadStatus::Confirmed,
                            ) {
                                Ok(_) => {
                                    debug!(
                                        "Pad {} (key {}) confirmed successfully.",
                                        current_pad_address, key_name
                                    );
                                    return;
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to update status to Confirmed for pad {} (key {}): {:?}. Aborting task.",
                                        current_pad_address, key_name, e
                                     );
                                    return;
                                }
                            }
                        } else {
                            error!(
                                "Pad {} (key {}) confirmation failed: Counter mismatch. Expected last_known={}, got current={}.",
                                current_pad_address, key_name, pad_to_confirm.last_known_counter, gotten_pad.counter
                            );
                        }
                    }
                    Err(e) => {
                        debug!(
                            "Error getting pad {} (key {}) for confirmation: {}",
                            current_pad_address, key_name, e
                        );
                    }
                }

                retries_left -= 1;
                if retries_left == 0 {
                    recycle_and_resend(
                        context.index.clone(),
                        pad_tx.clone(),
                        "persistent GET/confirmation error or counter mismatch".to_string(),
                        key_name.to_string(),
                        current_pad_address,
                    )
                    .await;
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        } else if status_after_put == PadStatus::Confirmed {
            debug!(
                "Pad {} (key {}) was already confirmed when checked before confirmation step.",
                current_pad_address, key_name
            );
        } else if status_after_put != PadStatus::Generated && status_after_put != PadStatus::Free {
            error!(
                "Pad {} (key {}) reached confirmation stage unexpectedly with status {:?}. Aborting task.",
                current_pad_address, key_name, status_after_put
            );
        }
        debug!(
            "process_pad_task: Finished processing pad {} for key {}",
            current_pad_address, key_name
        );
    }

    pub async fn get(&self, name: &str) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();

        let pads = self.index.read().await.get_pads(name);

        let mut tasks = Vec::new();
        for pad in pads {
            let network = self.network.clone();
            tasks.push(tokio::spawn(async move {
                let mut retries_left = 3;
                let pad = loop {
                    match network.get_private(&pad).await {
                        Ok(pad) => break pad,
                        Err(e) => {
                            debug!("Error getting pad {}: {}", pad.address, e);
                            retries_left -= 1;
                            if retries_left == 0 {
                                return Err(Error::Network(e));
                            }
                            continue;
                        }
                    };
                };

                Ok(pad.data)
            }));

            tokio::time::sleep(Duration::from_millis(100)).await;
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
}
