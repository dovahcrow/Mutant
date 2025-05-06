use std::sync::Arc;

use futures_util::{
    sink::SinkExt,
    stream::StreamExt,
};
use tokio::sync::mpsc;
use warp::ws::WebSocket;
use crate::TaskMap;
use mutant_lib::MutAnt;
use mutant_protocol::{ErrorResponse, Request, Response};

use super::common::send_response;
use super::dispatcher::handle_request;

pub async fn handle_ws(ws: WebSocket, mutant: Arc<MutAnt>, tasks: TaskMap) {
    let (mut ws_sender, mut ws_receiver) = ws.split();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<Response>();

    tracing::info!("WebSocket client connected");

    // Task to listen for updates from spawned tasks and send them to the client
    let update_forwarder = tokio::spawn(async move {
        while let Some(response) = update_rx.recv().await {
            if let Err(e) = send_response(&mut ws_sender, response).await {
                tracing::error!("Failed to send task update via WebSocket: {}", e);
                // If sending fails, the client might be disconnected, so we stop.
                break;
            }
        }
        // Ensure the sender is closed if the loop exits
        let _ = ws_sender.close().await;
        tracing::debug!("Update forwarder task finished.");
    });

    while let Some(result) = ws_receiver.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("WebSocket receive error: {}", e);
                // Don't need to send error here, forwarder handles closure
                break;
            }
        };

        if msg.is_close() {
            tracing::info!("WebSocket client disconnected explicitly");
            break;
        }

        if let Ok(text) = msg.to_str() {
            let original_request = text.to_string(); // Keep for error reporting
            match serde_json::from_str::<Request>(text) {
                Ok(request) => {
                    if let Err(e) = handle_request(
                        request,
                        original_request.as_str(),
                        update_tx.clone(), // Pass the update channel sender
                        mutant.clone(),
                        tasks.clone(),
                    )
                    .await
                    {
                        tracing::error!("Error handling request: {}", e);
                        let _ = update_tx.send(Response::Error(ErrorResponse {
                            error: e.to_string(),
                            original_request: Some(original_request),
                        }));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize request: {}", e);
                    let _ = update_tx.send(Response::Error(ErrorResponse {
                        error: format!("Invalid JSON request: {}", e),
                        original_request: Some(original_request),
                    }));
                }
            }
        } else if msg.is_binary() {
            tracing::warn!("Received binary message, ignoring.");
            let _ = update_tx.send(Response::Error(ErrorResponse {
                error: "Binary messages are not supported".to_string(),
                original_request: None,
            }));
        } else if msg.is_ping() {
            tracing::trace!("Received Ping");
        } else if msg.is_pong() {
            tracing::trace!("Received Pong");
        }
    }

    // Ensure the forwarder task is cleaned up when the receive loop ends
    update_forwarder.abort();
    tracing::debug!("WebSocket connection handler finished.");
}
