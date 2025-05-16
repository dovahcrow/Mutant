use mutant_protocol::Request;

use crate::{error::ClientError, MutantClient};

impl MutantClient {
    /// Sends a request over the WebSocket.
    pub async fn send_request(&mut self, request: Request) -> Result<(), ClientError> {
        log::info!("CLIENT: Attempting to send request: {:?}", request);

        // Check if we have a sender
        if self.sender.is_none() {
            log::error!("CLIENT: Cannot send request - WebSocket sender is None (not connected)");
            return Err(ClientError::NotConnected);
        }

        let sender = self.sender.as_mut().unwrap();

        // Serialize the request to JSON
        let json = match serde_json::to_string(&request) {
            Ok(json) => {
                log::info!("CLIENT: Successfully serialized request to JSON: {}", json);
                json
            },
            Err(e) => {
                log::error!("CLIENT: Failed to serialize request to JSON: {:?}", e);
                return Err(ClientError::SerializationError(e));
            }
        };

        // Use nash-ws to send the message
        log::info!("CLIENT: Sending WebSocket message");
        match sender.send(&nash_ws::Message::Text(json)).await {
            Ok(_) => {
                log::info!("CLIENT: Successfully sent WebSocket message");
                Ok(())
            },
            Err(e) => {
                let error_msg = format!("{:?}", e);
                log::error!("CLIENT: Failed to send WebSocket message: {}", error_msg);
                Err(ClientError::WebSocketError(error_msg))
            }
        }
    }
}
