use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    SinkExt,
};

use crate::utils::error::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt::Debug;
use {futures::stream::StreamExt, pharos::*, wasm_bindgen_futures::spawn_local, ws_stream_wasm::*};

fn protocol_from_bytes<P: 'static + Serialize + DeserializeOwned>(bytes: &[u8]) -> P {
    serde_cbor::from_slice(bytes).unwrap()
}

fn protocol_to_bytes<P: 'static + Serialize + DeserializeOwned>(packet: P) -> Vec<u8> {
    serde_cbor::to_vec(&packet).unwrap()
}

#[derive(Serialize, Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

pub struct Socket<P: 'static + Serialize + DeserializeOwned + Debug> {
    tx: UnboundedSender<P>,
    rx: Option<UnboundedReceiver<P>>,
}

impl<P: 'static + Serialize + DeserializeOwned + Debug> Socket<P>
where
    Self: 'static,
{
    fn new(tx: UnboundedSender<P>, rx: Option<UnboundedReceiver<P>>) -> Self {
        Self { tx, rx }
    }

    pub async fn connect(url: &str) -> Self {
        let (in_tx, mut in_rx) = futures::channel::mpsc::unbounded::<P>();
        let (mut out_tx, out_rx) = futures::channel::mpsc::unbounded();

        let (mut ws, wsio) = WsMeta::connect(url, None).await.expect("socket error");

        let evts = ws.observe(ObserveConfig::default()).await.unwrap();

        let _ = evts.for_each(|_evt| async move {});

        let (mut ws_tx, mut ws_rx) = wsio.split();

        let input_loop = async move {
            while let Some(msg) = in_rx.next().await {
                ws_tx
                    .send(WsMessage::Binary(protocol_to_bytes(msg)))
                    .await
                    .unwrap();
            }
        };

        spawn_local(input_loop);

        let output_loop = async move {
            while let Some(msg) = ws_rx.next().await {
                if let WsMessage::Binary(blob) = msg {
                    let msg = protocol_from_bytes(&blob);

                    out_tx.send(msg).await.unwrap();
                } else {
                    // bad message type
                }
            }
        };

        spawn_local(output_loop);

        Self::new(in_tx, Some(out_rx))
    }

    pub async fn send(&mut self, packet: P) -> Result<()> {
        self.tx.send(packet).await?;

        Ok(())
    }

    pub fn take_receiver(&mut self) -> Option<UnboundedReceiver<P>> {
        self.rx.take()
    }
}
