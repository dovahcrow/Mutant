use std::sync::Arc;
use futures_util::{sink::SinkExt, stream::StreamExt};
use warp::ws::{WebSocket, Message};
use mutant_lib::{MutAnt, storage::ScratchpadAddress};
use super::transcoding::{needs_transcoding, MpegTsTranscoder};

/// Handle video streaming WebSocket connections
/// This endpoint streams video data for all video formats with appropriate transcoding
pub async fn handle_video_stream(ws: WebSocket, mutant: Arc<MutAnt>, filename: String) {
    let (mut ws_sender, _ws_receiver) = ws.split();

    log::info!("Video streaming client connected for file: {}", filename);

    // URL decode the filename to get the actual key name
    let user_key = match urlencoding::decode(&filename) {
        Ok(decoded) => decoded.to_string(),
        Err(e) => {
            log::error!("Failed to URL decode filename '{}': {}", filename, e);
            filename.clone() // Fallback to original filename
        }
    };

    log::info!("URL decoded key: '{}' (from '{}')", user_key, filename);

    log::info!("Attempting to stream video for key: '{}'", user_key);

    // Check if this format needs transcoding
    let needs_transcode = needs_transcoding(&filename);
    log::info!("Video format transcoding needed for '{}': {}", filename, needs_transcode);

    // Start streaming the video data with appropriate handling
    match stream_video_data(&mut ws_sender, &mutant, &user_key, needs_transcode).await {
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

/// Helper function to try getting data with immediate pad streaming
async fn try_get_with_immediate_streaming(
    mutant: &MutAnt,
    user_key: &str,
) -> Result<(tokio::sync::mpsc::UnboundedReceiver<(usize, Vec<u8>)>, bool), Box<dyn std::error::Error + Send + Sync>> {
    // First try as a private key
    match mutant.get_with_immediate_streaming(user_key, None).await {
        Ok(pad_rx) => {
            log::info!("VIDEO STREAM: Successfully started immediate streaming for private key: {}", user_key);
            Ok((pad_rx, false)) // false = private
        }
        Err(e) => {
            log::info!("VIDEO STREAM: Failed to get as private key for '{}': {}. Trying public access...", user_key, e);

            // Try to get the public address for this key
            match mutant.get_public_index_address(user_key).await {
                Ok(public_address_hex) => {
                    log::info!("VIDEO STREAM: Got public address for key '{}': {}", user_key, public_address_hex);

                    // Parse the hex address to ScratchpadAddress
                    match ScratchpadAddress::from_hex(&public_address_hex) {
                        Ok(address) => {
                            // Try public immediate streaming
                            match mutant.get_public_with_immediate_streaming(&address, None).await {
                                Ok(pad_rx) => {
                                    log::info!("VIDEO STREAM: Successfully started immediate streaming for public key: {}", user_key);
                                    Ok((pad_rx, true)) // true = public
                                }
                                Err(e) => {
                                    log::error!("VIDEO STREAM: Failed to get public data with immediate streaming for '{}': {}", user_key, e);
                                    Err(format!("Failed to get public data with immediate streaming: {}", e).into())
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("VIDEO STREAM: Invalid public address hex for '{}': {}", user_key, e);
                            Err(format!("Invalid public address hex: {}", e).into())
                        }
                    }
                }
                Err(e) => {
                    log::error!("VIDEO STREAM: Failed to get public address for '{}': {}", user_key, e);
                    Err(format!("Failed to get public address: {}", e).into())
                }
            }
        }
    }
}

/// Stream video data from MutAnt to the WebSocket client using true streaming
/// Supports both direct streaming and transcoding based on format requirements
async fn stream_video_data(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    mutant: &MutAnt,
    user_key: &str,
    needs_transcoding: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting immediate pad streaming for key: {} (transcoding: {})", user_key, needs_transcoding);

    if needs_transcoding {
        // For formats that need transcoding, we need to download the full video first
        // then transcode it to MPEG-TS for streaming
        return stream_transcoded_video_data(ws_sender, mutant, user_key).await;
    }

    // For native formats, use immediate pad streaming
    log::info!("VIDEO STREAM: Using immediate pad streaming for key: {}", user_key);

    // Try to get the data with immediate pad streaming
    let (mut pad_rx, is_public) = match try_get_with_immediate_streaming(mutant, user_key).await {
        Ok((pad_rx, is_public)) => (pad_rx, is_public),
        Err(e) => {
            log::error!("VIDEO STREAM: Failed to start immediate streaming for '{}': {}", user_key, e);
            return Err(format!("Failed to start streaming: {}", e).into());
        }
    };

    log::info!("VIDEO STREAM: Successfully started immediate pad streaming for key: {} (public: {})", user_key, is_public);

    // Stream pads to WebSocket as they arrive immediately
    let mut total_bytes_sent = 0;
    const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for WebSocket

    // Receive and forward pads immediately as they arrive
    while let Some((pad_index, pad_data)) = pad_rx.recv().await {
        log::info!("VIDEO STREAM: Received pad {} ({} bytes) - forwarding immediately to WebSocket", pad_index, pad_data.len());

        // Break large pads into smaller WebSocket messages if needed
        let mut offset = 0;
        while offset < pad_data.len() {
            let end = std::cmp::min(offset + CHUNK_SIZE, pad_data.len());
            let sub_chunk = &pad_data[offset..end];

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

    // All pads have been streamed
    log::info!("VIDEO STREAM: Completed immediate pad streaming {} bytes for key: {}", total_bytes_sent, user_key);
    Ok(())
}

/// Stream transcoded video data for formats that need transcoding
async fn stream_transcoded_video_data(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    mutant: &MutAnt,
    user_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting transcoded video streaming for key: {}", user_key);

    // First, download the complete video data
    let video_data = get_complete_video_data(mutant, user_key).await?;
    log::info!("Downloaded {} bytes for transcoding: {}", video_data.len(), user_key);

    // Start the MPEG-TS transcoder
    let mut transcoder = MpegTsTranscoder::new(video_data).await?;
    log::info!("Started MPEG-TS transcoder for: {}", user_key);

    // Stream the transcoded data to WebSocket
    let mut total_bytes_sent = 0;
    const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for WebSocket
    let mut buffer = vec![0u8; CHUNK_SIZE];

    loop {
        use tokio::io::AsyncReadExt;

        match transcoder.read(&mut buffer).await {
            Ok(0) => {
                // End of stream
                log::info!("Transcoding completed for '{}', sent {} bytes total", user_key, total_bytes_sent);
                break;
            }
            Ok(bytes_read) => {
                let chunk = &buffer[..bytes_read];

                // Send the transcoded chunk as binary data
                match ws_sender.send(Message::binary(chunk)).await {
                    Ok(_) => {
                        log::debug!("Sent transcoded chunk ({} bytes) for key: {}", bytes_read, user_key);
                        total_bytes_sent += bytes_read;
                    }
                    Err(e) => {
                        log::error!("Failed to send transcoded WebSocket chunk for key '{}': {}", user_key, e);
                        return Err(format!("WebSocket send error: {}", e).into());
                    }
                }

                // Small delay to prevent overwhelming the client
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }
            Err(e) => {
                log::error!("Transcoding read error for key '{}': {}", user_key, e);
                return Err(format!("Transcoding error: {}", e).into());
            }
        }
    }

    Ok(())
}

/// Download complete video data from MutAnt (used for transcoding)
async fn get_complete_video_data(
    mutant: &MutAnt,
    user_key: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Downloading complete video data for transcoding: {}", user_key);

    // First try as a private key
    match mutant.get(user_key, None, false).await {
        Ok(data) => {
            log::info!("Retrieved video data as private key: {} ({} bytes)", user_key, data.len());
            Ok(data)
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
                                    Ok(data)
                                }
                                Err(e) => {
                                    log::error!("Failed to get public data for '{}': {}", user_key, e);
                                    Err(format!("Failed to get public data: {}", e).into())
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Invalid public address hex for '{}': {}", user_key, e);
                            Err(format!("Invalid public address hex: {}", e).into())
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to get public address for '{}': {}", user_key, e);
                    Err(format!("Failed to get public address: {}", e).into())
                }
            }
        }
    }
}
