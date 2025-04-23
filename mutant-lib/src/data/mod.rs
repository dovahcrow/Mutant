use crate::{
    index::{
        master_index::{IndexEntry, MasterIndex},
        PadInfo, PadStatus, DEFAULT_SCRATCHPAD_SIZE,
    },
    network::Network,
};
use error::DataError;
use log::{debug, error, info, warn};
use std::{intrinsics::drop_in_place, sync::Arc, time::Duration};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::RwLock;

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

pub const CHUNK_CONFIRMATION_QUEUE_SIZE: usize = 256;

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
        let (put_tx, put_rx) = channel(CHUNK_CONFIRMATION_QUEUE_SIZE);
        let (confirm_tx, confirm_rx) = channel(CHUNK_CONFIRMATION_QUEUE_SIZE);
        let key = context.name.clone();
        debug!("write_pipeline: Starting pipeline for key {}", key);

        // prepare futures for put and confirm stages
        let put_future =
            self.put_private_pads(context.clone(), put_tx.clone(), put_rx, confirm_tx.clone());
        let confirm_future = self.confirm_private_pads(context.clone(), put_tx.clone(), confirm_rx);

        // send initial pads
        debug!(
            "write_pipeline: Sending {} pads for key {}",
            pads.len(),
            key
        );
        for pad in pads {
            if put_tx.send(pad).await.is_err() {
                warn!("write_pipeline: failed to send pad; put_rx closed");
            }
        }

        // close initial channels to signal completion
        drop(put_tx);
        drop(confirm_tx);

        // run put and confirm stages concurrently
        let (_put_res, _confirm_res) = tokio::join!(put_future, confirm_future);
        debug!("write_pipeline: Completed pipeline for key {}", key);
    }

    // pub async fn store_public(&self, name: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

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
            let pad_data = result.unwrap();
            let pad_data = pad_data.unwrap();
            data.extend_from_slice(&pad_data);
        }

        Ok(data)
    }

    // pub async fn get_public(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, name: &[u8]) -> Result<(), Error> {}

    async fn put_private_pads(
        &self,
        context: Context,
        put_tx: Sender<PadInfo>,
        mut put_rx: Receiver<PadInfo>,
        confirm_tx: Sender<PadInfo>,
    ) {
        let handle = tokio::task::spawn(async move {
            let mut tasks = Vec::new();

            while let Some(pad) = put_rx.recv().await {
                if pad.status != PadStatus::Generated && pad.status != PadStatus::Free {
                    debug!(
                        "Skipping pad write for {} because it's not generated or free. It's {:?}",
                        pad.address, pad.status
                    );
                    confirm_tx.send(pad).await.unwrap();
                    continue;
                }

                let context = context.clone();
                let confirm_tx = confirm_tx.clone();
                let put_tx = put_tx.clone();

                tasks.push(tokio::task::spawn(async move {
                    let mut retries_left = 3;
                    let put_result = loop {
                        let put_result = context
                            .network
                            .put_private(
                                &pad,
                                &context.chunks[pad.chunk_index],
                                DATA_ENCODING_PRIVATE_DATA,
                            )
                            .await;

                        if let Err(e) = put_result {
                            debug!("Error putting pad {}: {}", pad.address, e);
                            retries_left -= 1;
                            if retries_left == 0 {
                                warn!("Recycling pad {} because of put error", pad.address);

                                let new_pad = context
                                    .index
                                    .write()
                                    .await
                                    .recycle_errored_pad(&context.name, &pad.address)
                                    .unwrap();

                                debug!("Sent new pad {} to confirmation queue", new_pad.address);

                                put_tx.send(new_pad).await.unwrap();
                                return;
                            } else {
                                continue;
                            }
                        } else {
                            break put_result;
                        }
                    };

                    let pad = context
                        .index
                        .write()
                        .await
                        .update_pad_status(&context.name, &pad.address, PadStatus::Written)
                        .unwrap();

                    let pad_address = pad.address;
                    debug!(
                        "put_private_pads: Sub-task for pad {} sending to confirm_tx...",
                        pad_address
                    );
                    confirm_tx.send(pad).await.unwrap();
                    debug!("put_private_pads: Sub-task finished sending confirmation");
                }));
            }
            debug!("put_private_pads: Main recv loop finished.");

            debug!("put_private_pads: Dropping internal sender clones.");
            drop(confirm_tx);
            drop(put_tx);

            debug!(
                "put_private_pads: Main loop finished, joining {} sub-tasks...",
                tasks.len()
            );
            let results = futures::future::join_all(tasks).await;
            debug!("put_private_pads: Finished joining sub-tasks.");
            for result in results {
                if let Err(e) = result {
                    error!("put_private_pads sub-task failed: {:?}", e);
                }
            }
            debug!("put_private_pads: All sub-tasks joined.");
            debug!("All pads written for {}", context.name);
        });
        if let Err(e) = handle.await {
            error!("put_private_pads main task panicked: {:?}", e);
        }
    }

    async fn confirm_private_pads(
        &self,
        context: Context,
        put_tx: Sender<PadInfo>,
        mut confirm_rx: Receiver<PadInfo>,
    ) {
        let handle = tokio::task::spawn(async move {
            let mut tasks = Vec::new();

            while let Some(pad) = confirm_rx.recv().await {
                if pad.status != PadStatus::Written {
                    debug!(
                        "Skipping pad confirmation for {} because it's not written. It's {:?}",
                        pad.address, pad.status
                    );
                    continue;
                }

                let context = context.clone();
                let put_tx = put_tx.clone();

                tasks.push(tokio::task::spawn(async move {
                    let mut retries_left = 3;
                    loop {
                        let gotten_pad = match context.network.get_private(&pad).await {
                            Ok(p) => p,
                            Err(e) => {
                                debug!("Error getting pad {}: {}", pad.address, e);
                                retries_left -= 1;
                                if retries_left == 0 {
                                    warn!(
                                        "Recycling pad {} because of get error after a put",
                                        pad.address
                                    );

                                    let new_pad = context
                                        .index
                                        .write()
                                        .await
                                        .recycle_errored_pad(&context.name, &pad.address)
                                        .unwrap();

                                    let _ = put_tx.send(new_pad).await;
                                    drop(put_tx);
                                    return;
                                } else {
                                    continue;
                                }
                            }
                        };

                        if (pad.last_known_counter == 0 && gotten_pad.counter == 0)
                            || pad.last_known_counter <= gotten_pad.counter
                        {
                            let pad = context
                                .index
                                .write()
                                .await
                                .update_pad_status(
                                    &context.name,
                                    &pad.address,
                                    PadStatus::Confirmed,
                                )
                                .unwrap();

                            drop(put_tx);
                            debug!(
                                "confirm_private_pads: Sub-task finished for pad {}",
                                pad.address
                            );
                            return;
                        }

                        if pad.last_known_counter < gotten_pad.counter {
                            error!(
                                "Pad {} has a counter of {} but got {}",
                                pad.address, pad.last_known_counter, gotten_pad.counter
                            );
                        }
                    }
                }));
            }
            debug!("confirm_private_pads: Main recv loop finished.");

            debug!("confirm_private_pads: Dropping internal put_tx sender clone.");
            drop(put_tx);

            debug!(
                "confirm_private_pads: Main loop finished, joining {} sub-tasks...",
                tasks.len()
            );
            let results = futures::future::join_all(tasks).await;
            debug!("confirm_private_pads: Finished joining sub-tasks.");
            for result in results {
                result.unwrap();
            }
            debug!("confirm_private_pads: All sub-tasks joined.");
            debug!("All pads confirmed for {}", context.name);
        });
        if let Err(e) = handle.await {
            error!("confirm_private_pads main task panicked: {:?}", e);
        }
    }
}
