use std::sync::Arc;
use futures_util::{sink::SinkExt, stream::StreamExt};
use warp::ws::{WebSocket, Message};
use mutant_lib::{MutAnt, storage::ScratchpadAddress};

/// Handle video streaming WebSocket connections
/// This endpoint streams video data directly to mpegts.js players
pub async fn handle_video_stream(ws: WebSocket, mutant: Arc<MutAnt>, filename: String) {
    let (mut ws_sender, _ws_receiver) = ws.split();
    
    log::info!("Video streaming client connected for file: {}", filename);

    // Use the full filename as the key (including extension)
    let user_key = filename.clone();

    log::info!("Attempting to stream video for key: '{}'", user_key);

    // Start streaming the video data
    match stream_video_data(&mut ws_sender, &mutant, &user_key).await {
        Ok(_) => {
            log::info!("Video streaming completed successfully for: {}", filename);
        }
        Err(e) => {
            log::error!("Video streaming failed for {}: {}", filename, e);
            // Don't send error messages as text - mpegts.js expects only binary data
            // The connection will be closed which will signal the error to the client
        }
    }
    
    // Close the connection
    let _ = ws_sender.close().await;
    log::info!("Video streaming connection closed for: {}", filename);
}

/// Stream video data from MutAnt to the WebSocket client using true streaming
async fn stream_video_data(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    mutant: &MutAnt,
    user_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::sync::mpsc;
    use mutant_lib::events::{GetCallback, GetEvent};

    log::info!("Starting streaming video data for key: {}", user_key);

    // Create a channel to receive streaming data
    let (data_tx, mut data_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let data_tx_clone = data_tx.clone();

    // Create a callback that sends data chunks as they arrive
    let callback: GetCallback = Arc::new(move |event| {
        let data_tx = data_tx_clone.clone();
        Box::pin(async move {
            match event {
                GetEvent::PadData { chunk_index, data } => {
                    log::info!("VIDEO STREAM: Received chunk {} ({} bytes)", chunk_index, data.len());

                    // Send the data immediately to the streaming channel
                    if let Err(_) = data_tx.send(data) {
                        log::error!("VIDEO STREAM: Failed to send chunk {} to streaming channel", chunk_index);
                        return Ok(false); // Stop streaming
                    }

                    Ok(true) // Continue streaming
                }
                GetEvent::Complete => {
                    log::info!("VIDEO STREAM: Download completed");
                    Ok(true)
                }
                _ => Ok(true), // Continue for other events
            }
        })
    });

    // Start the streaming download in a background task
    let mutant_clone = mutant.clone();
    let user_key_clone = user_key.to_string();
    let data_tx_for_task = data_tx.clone();
    let download_task = tokio::spawn(async move {
        // First try as a private key with streaming enabled
        log::info!("VIDEO STREAM: Attempting to get video data for key '{}' as private key with streaming", user_key_clone);
        match mutant_clone.get(&user_key_clone, Some(callback), true).await {
            Ok(_) => {
                log::info!("VIDEO STREAM: Successfully started streaming for private key: {}", user_key_clone);
                Ok(())
            }
            Err(e) => {
                log::info!("VIDEO STREAM: Failed to get as private key for '{}': {}. Trying public access...", user_key_clone, e);

                // Try to get the public address for this key
                match mutant_clone.get_public_index_address(&user_key_clone).await {
                    Ok(public_address_hex) => {
                        log::info!("VIDEO STREAM: Got public address for key '{}': {}", user_key_clone, public_address_hex);

                        // Parse the hex address to ScratchpadAddress
                        match ScratchpadAddress::from_hex(&public_address_hex) {
                            Ok(address) => {
                                // Create a new callback for public access
                                let (pub_data_tx, mut pub_data_rx) = mpsc::unbounded_channel::<Vec<u8>>();

                                let pub_callback: GetCallback = Arc::new(move |event| {
                                    let pub_data_tx = pub_data_tx.clone();
                                    Box::pin(async move {
                                        match event {
                                            GetEvent::PadData { chunk_index, data } => {
                                                log::info!("VIDEO STREAM PUBLIC: Received chunk {} ({} bytes)", chunk_index, data.len());

                                                if let Err(_) = pub_data_tx.send(data) {
                                                    log::error!("VIDEO STREAM PUBLIC: Failed to send chunk {} to streaming channel", chunk_index);
                                                    return Ok(false);
                                                }

                                                Ok(true)
                                            }
                                            GetEvent::Complete => {
                                                log::info!("VIDEO STREAM PUBLIC: Download completed");
                                                Ok(true)
                                            }
                                            _ => Ok(true),
                                        }
                                    })
                                });

                                // Fetch the public data with streaming
                                match mutant_clone.get_public(&address, Some(pub_callback), true).await {
                                    Ok(_) => {
                                        log::info!("VIDEO STREAM: Successfully started streaming for public key: {}", user_key_clone);

                                        // Forward data from public channel to main channel
                                        while let Some(chunk) = pub_data_rx.recv().await {
                                            if data_tx_for_task.send(chunk).is_err() {
                                                break;
                                            }
                                        }

                                        Ok(())
                                    }
                                    Err(e) => {
                                        log::error!("VIDEO STREAM: Failed to get public data for '{}': {}", user_key_clone, e);
                                        Err(format!("Failed to get public data: {}", e))
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("VIDEO STREAM: Invalid public address hex for '{}': {}", user_key_clone, e);
                                Err(format!("Invalid public address hex: {}", e))
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("VIDEO STREAM: Failed to get public address for '{}': {}", user_key_clone, e);
                        Err(format!("Failed to get public address: {}", e))
                    }
                }
            }
        }
    });

    // Stream data to WebSocket as it arrives
    let mut total_bytes_sent = 0;
    const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for WebSocket

    // Close the original sender to signal completion when download finishes
    drop(data_tx);

    while let Some(chunk_data) = data_rx.recv().await {
        log::info!("VIDEO STREAM: Streaming chunk ({} bytes) to WebSocket", chunk_data.len());

        // Break large chunks into smaller WebSocket messages if needed
        let mut offset = 0;
        while offset < chunk_data.len() {
            let end = std::cmp::min(offset + CHUNK_SIZE, chunk_data.len());
            let sub_chunk = &chunk_data[offset..end];

            // Send the chunk as binary data
            match ws_sender.send(Message::binary(sub_chunk)).await {
                Ok(_) => {
                    log::debug!("VIDEO STREAM: Sent WebSocket chunk {}-{} ({} bytes) for key: {}",
                               offset, end, sub_chunk.len(), user_key);
                    total_bytes_sent += sub_chunk.len();
                }
                Err(e) => {
                    log::error!("VIDEO STREAM: Failed to send WebSocket chunk for key '{}': {}", user_key, e);
                    return Err(format!("WebSocket send error: {}", e).into());
                }
            }

            offset = end;

            // Small delay to prevent overwhelming the client
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    // Wait for the download task to complete
    match download_task.await {
        Ok(Ok(())) => {
            log::info!("VIDEO STREAM: Completed streaming {} bytes for key: {}", total_bytes_sent, user_key);
            Ok(())
        }
        Ok(Err(e)) => {
            log::error!("VIDEO STREAM: Download task failed for key '{}': {}", user_key, e);
            Err(e.into())
        }
        Err(e) => {
            log::error!("VIDEO STREAM: Download task panicked for key '{}': {}", user_key, e);
            Err(format!("Download task panicked: {}", e).into())
        }
    }
}
