use futures_util::sink::SinkExt;
use tokio::sync::mpsc;
use warp::ws::{Message, WebSocket};

use crate::error::Error as DaemonError;
use mutant_protocol::Response;

/// Helper function to send JSON responses
pub(crate) async fn send_response(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    response: Response,
) -> Result<(), DaemonError> {
    let json = serde_json::to_string(&response).map_err(DaemonError::SerdeJson)?;
    sender
        .send(Message::text(json))
        .await
        .map_err(DaemonError::WebSocket)?;
    Ok(())
}

/// Type alias for the update channel sender
pub(crate) type UpdateSender = mpsc::UnboundedSender<Response>;
