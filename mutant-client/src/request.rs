use mutant_protocol::Request;

use crate::{error::ClientError, MutantClient};

impl MutantClient {
    /// Sends a request over the WebSocket.
    pub async fn send_request(&mut self, request: Request) -> Result<(), ClientError> {
        let sender = self
            .sender
            .as_mut()
            .ok_or_else(|| ClientError::NotConnected)?;

        let json =
            serde_json::to_string(&request).map_err(|e| ClientError::SerializationError(e))?;

        // Use nash-ws to send the message
        sender.send(&nash_ws::Message::Text(json))
            .await
            .map_err(|e| ClientError::WebSocketError(format!("{:?}", e)))?;

        Ok(())
    }
}
