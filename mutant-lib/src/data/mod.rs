use crate::{
    index::{
        master_index::{IndexEntry, MasterIndex},
        PadInfo, PadStatus, DEFAULT_SCRATCHPAD_SIZE,
    },
    network::Network,
};
use error::DataError;
use std::sync::Arc;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{broadcast::channel, RwLock};

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

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
        // determine if this is
        // - A first store on that key
        //    - If so, we want to determine the number of pads needed by splitting the data
        //    - We create the key in the master index to get a list of the pads
        //    - We start the process to write the data to the pads in parallel for each pad
        //        - Once the pad has been written to with the network::store_private() function, we update the pad info in the master index
        //        - This should include a confirmation process for each pad: we fetch the pad and we check the counter:
        //            - While the pad is not found or the counter is less than the one we just put, we loop. Let's make it 32 times max
        //            - The WHOLE process MUST BE DONE IN PARALLEL for each pad. No waiting for anything
        // - An update on that key if the checksum mismatch (wether or not the previous upload was done)
        //    - This is basically just a remove followed by a store
        // - A resume on that key because some pads are not Confirmed (if the data is different, it will be an update, if the data is the same, resume the process for the missing pads only)
        //    - We check which pads are Generated or Allocated and we start the full private store process for those missing pads only.
        //       - This should include the confirmation process for each of those pads, maybe make a queue to process them all in parallel with the rest (so that we have one big pipeline)
        //    - For the written pads, we just go through the confirmation process
        //    - No need to do anything for the Confirmed pads

        if self.index.read().await.contains_key(&name) {
            if self.index.read().await.verify_checksum(&name, data_bytes) {
                // it's a resume
                self.resume(name, data_bytes).await
            } else {
                // it's an update
                self.index.write().await.remove_key(&name).unwrap();

                return self.first_store(name, data_bytes).await;
            }
        } else {
            self.first_store(name, data_bytes).await
        }
    }

    async fn resume(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
        // it's a resume
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
        // it's a first store
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
        let (confirm_tx, confirm_rx) = channel(32);
        self.put_private_pads(context.clone(), pads, confirm_tx)
            .await;
        self.confirm_private_pads(context, confirm_rx).await;
    }

    async fn put_private_pads(
        &self,
        context: Context,
        pads: Vec<PadInfo>,
        confirm_tx: Sender<PadInfo>,
    ) {
        for (i, pad) in pads.into_iter().enumerate() {
            if pad.status != PadStatus::Generated && pad.status != PadStatus::Free {
                confirm_tx.send(pad).unwrap();
                continue;
            }

            let context = context.clone();
            let confirm_tx = confirm_tx.clone();
            tokio::task::spawn(async move {
                context
                    .network
                    .put_private(&pad, &context.chunks[i], DATA_ENCODING_PRIVATE_DATA)
                    .await
                    .unwrap();

                let pad = context
                    .index
                    .write()
                    .await
                    .update_pad_status(&context.name, &pad.address, PadStatus::Written)
                    .unwrap();

                confirm_tx.send(pad).unwrap();
            });
        }

        drop(confirm_tx); // Ensure the original sender is dropped
    }

    async fn confirm_private_pads(&self, context: Context, mut confirm_rx: Receiver<PadInfo>) {
        let mut tasks = Vec::new();

        while let Ok(pad) = confirm_rx.recv().await {
            if pad.status != PadStatus::Written {
                continue;
            }

            let context = context.clone();
            tasks.push(tokio::task::spawn(async move {
                loop {
                    let gotten_pad = context.network.get_private(&pad).await.unwrap();

                    if pad.last_known_counter == gotten_pad.counter {
                        let pad = context
                            .index
                            .write()
                            .await
                            .update_pad_status(&context.name, &pad.address, PadStatus::Confirmed)
                            .unwrap();

                        return pad;
                    }
                }
            }));
        }

        for task in tasks {
            let pad = task.await.unwrap();
        }
    }

    // pub async fn store_public(&self, name: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

    pub async fn get(&self, name: &str) -> Result<Vec<u8>, Error> {
        let mut data = Vec::new();

        let pads = self.index.read().await.get_pads(name);

        let mut tasks = Vec::new();
        for pad in pads {
            let network = self.network.clone();
            tasks.push(tokio::spawn(async move {
                network.get_private(&pad).await.unwrap().data
            }));
        }

        let results = futures::future::join_all(tasks).await;
        for result in results {
            let pad_data = result.unwrap();
            data.extend_from_slice(&pad_data);
        }

        Ok(data)
    }

    // pub async fn get_public(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, name: &[u8]) -> Result<(), Error> {}
}
