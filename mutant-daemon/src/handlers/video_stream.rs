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

/// Stream video data from MutAnt to the WebSocket client
async fn stream_video_data(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    mutant: &MutAnt,
    user_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting video data stream for key: {}", user_key);
    
    // Try to get the video data from MutAnt
    // First try as a regular key, then try to get the public address and fetch it
    log::info!("Attempting to get video data for key '{}' as private key", user_key);
    let video_data = match mutant.get(user_key, None, false).await {
        Ok(data) => {
            log::info!("Successfully retrieved video data as private key: {} ({} bytes)", user_key, data.len());
            data
        }
        Err(e) => {
            log::info!("Failed to get as private key for '{}': {}. Trying public access...", user_key, e);
            // Try to get the public address for this key
            match mutant.get_public_index_address(user_key).await {
                Ok(public_address_hex) => {
                    log::info!("Got public address for key '{}': {}", user_key, public_address_hex);
                    // Parse the hex address to ScratchpadAddress
                    match ScratchpadAddress::from_hex(&public_address_hex) {
                        Ok(address) => {
                            // Fetch the public data
                            match mutant.get_public(&address, None, false).await {
                                Ok(data) => {
                                    log::info!("Retrieved video data as public key: {} ({} bytes)", user_key, data.len());
                                    data
                                }
                                Err(e) => {
                                    let error_msg = format!("Failed to retrieve public video data for key '{}': {}", user_key, e);
                                    log::error!("{}", error_msg);
                                    return Err(error_msg.into());
                                }
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse public address for key '{}': {}", user_key, e);
                            log::error!("{}", error_msg);
                            return Err(error_msg.into());
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Failed to get public address for key '{}': {}", user_key, e);
                    log::error!("{}", error_msg);
                    log::info!("Key '{}' not found. Make sure the file is uploaded to MutAnt storage first.", user_key);
                    return Err(error_msg.into());
                }
            }
        }
    };
    
    log::info!("Streaming {} bytes of video data for key: {}", video_data.len(), user_key);
    
    // Stream the video data in chunks
    const CHUNK_SIZE: usize = 262_144; // 256KB chunks for smooth streaming
    let mut offset = 0;
    
    while offset < video_data.len() {
        let end = std::cmp::min(offset + CHUNK_SIZE, video_data.len());
        let chunk = &video_data[offset..end];
        
        // Send the chunk as binary data
        match ws_sender.send(Message::binary(chunk)).await {
            Ok(_) => {
                log::debug!("Sent video chunk {}-{} ({} bytes) for key: {}", 
                           offset, end, chunk.len(), user_key);
            }
            Err(e) => {
                log::error!("Failed to send video chunk for key '{}': {}", user_key, e);
                return Err(format!("WebSocket send error: {}", e).into());
            }
        }
        
        offset = end;
        
        // Small delay to prevent overwhelming the client
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
    
    log::info!("Completed streaming {} bytes for key: {}", video_data.len(), user_key);
    Ok(())
}
