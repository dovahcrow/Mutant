use crate::{
    index::{master_index::MasterIndex, PadStatus},
    network::Network,
};
use error::DataError;
use std::sync::Arc;
use tokio::sync::{broadcast::channel, RwLock};

pub const DATA_ENCODING_MASTER_INDEX: u64 = 0;
pub const DATA_ENCODING_PRIVATE_DATA: u64 = 1;
pub const DATA_ENCODING_PUBLIC_INDEX: u64 = 2;
pub const DATA_ENCODING_PUBLIC_DATA: u64 = 3;

mod error;

pub use crate::internal_error::Error;

pub struct Data {
    network: Arc<Network>,
    index: Arc<RwLock<MasterIndex>>,
}

impl Data {
    pub fn new(network: Arc<Network>, index: Arc<RwLock<MasterIndex>>) -> Self {
        Self { network, index }
    }

    pub async fn store(&self, name: &str, data_bytes: &[u8]) -> Result<(), Error> {
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
                return Ok(());
            } else {
                // it's an update
                return Ok(());
            }
        } else {
            // it's a first store
            let pads = self
                .index
                .write()
                .await
                .create_private_key(&name, data_bytes)?;

            let (confirm_tx, mut confirm_rx) = channel(32);

            let name = name.to_string();
            let data_bytes = data_bytes.to_vec();
            for pad in pads {
                let index = self.index.clone();
                let network = self.network.clone();
                let data_bytes2 = data_bytes.clone();
                let name = name.clone();
                let confirm_tx = confirm_tx.clone();
                tokio::task::spawn(async move {
                    let index = index.clone();
                    let network = network.clone();
                    let data_bytes2 = data_bytes2.clone();
                    let name = name.clone();
                    network
                        .put_private(&pad, &data_bytes2, DATA_ENCODING_PRIVATE_DATA)
                        .await
                        .unwrap();

                    index
                        .write()
                        .await
                        .update_pad_status(&name, &pad.address, PadStatus::Written);

                    confirm_tx.send(pad);
                });
            }

            let (finished_tx, mut finished_rx) = channel(32);

            let index = self.index.clone();
            let network = self.network.clone();

            let name = name.to_string();

            tokio::task::spawn(async move {
                while let Ok(pad) = confirm_rx.recv().await {
                    let index = index.clone();
                    let network = network.clone();
                    let name = name.clone();
                    let finished_tx = finished_tx.clone();
                    tokio::task::spawn(async move {
                        loop {
                            let gotten_pad = network.get_private(&pad).await.unwrap();

                            if pad.last_known_counter == gotten_pad.counter {
                                index.write().await.update_pad_status(
                                    &name.clone(),
                                    &pad.address,
                                    PadStatus::Confirmed,
                                );

                                finished_tx.send(pad);
                                break;
                            }
                        }
                    });
                }
            });

            while let Ok(pad) = finished_rx.recv().await {
                println!("Pad confirmed: {:?}", pad);
            }

            println!("All pads confirmed");

            Ok(())
        }
    }

    // pub async fn store_public(&self, name: &[u8], data_bytes: &[u8]) -> Result<(), Error> {}

    // pub async fn get(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn get_public(&self, name: &[u8]) -> Result<Vec<u8>, Error> {}

    // pub async fn remove(&self, name: &[u8]) -> Result<(), Error> {}
}
