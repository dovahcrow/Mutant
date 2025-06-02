use std::sync::Arc;
use warp::{Reply, Rejection, http::StatusCode};
use mutant_lib::{MutAnt, storage::ScratchpadAddress};
use futures_util::StreamExt;
use super::transcoding::{needs_transcoding, get_transcoded_mime_type, TranscodedVideoStream};

/// Handle HTTP video requests with range support and transcoding for unsupported formats
pub async fn handle_http_video(
    filename: String,
    range_header: Option<String>,
    mutant: Arc<MutAnt>,
) -> Result<impl Reply, Rejection> {
    log::info!("HTTP video request for file: {}, range: {:?}", filename, range_header);

    // Use the full filename as the key (including extension)
    let user_key = filename.clone();

    // Get the video data from MutAnt
    let video_data = match get_video_data(&mutant, &user_key).await {
        Ok(data) => data,
        Err(e) => {
            log::error!("Failed to get video data for {}: {}", filename, e);
            return Ok(warp::reply::with_status(
                format!("Video not found: {}", e),
                StatusCode::NOT_FOUND,
            ).into_response());
        }
    };

    log::info!("Retrieved video data for {}: {} bytes", filename, video_data.len());

    // Check if transcoding is needed
    if needs_transcoding(&filename) {
        log::info!("Video format requires transcoding: {}", filename);
        return handle_transcoded_video(filename, range_header, video_data).await;
    }

    // Handle native browser-supported formats with range support
    handle_native_video(filename, range_header, video_data)
}

/// Handle transcoded video streaming with range support
async fn handle_transcoded_video(
    filename: String,
    range_header: Option<String>,
    video_data: Vec<u8>,
) -> Result<warp::reply::Response, Rejection> {
    log::info!("Starting transcoding for video: {}", filename);

    // Create transcoded video stream
    let transcoded_stream = match TranscodedVideoStream::new(video_data).await {
        Ok(stream) => stream,
        Err(e) => {
            log::error!("Failed to start transcoding for {}: {}", filename, e);
            return Ok(warp::reply::with_status(
                format!("Transcoding failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ).into_response());
        }
    };

    // For transcoded content, we don't support range requests initially
    // The browser will get the full stream and handle buffering
    if range_header.is_some() {
        log::info!("Range request for transcoded content not supported, serving full stream for {}", filename);
    }

    log::info!("Starting streaming response for transcoded video: {}", filename);

    // Convert our stream to a warp-compatible stream
    let body_stream = transcoded_stream.map(|chunk_result| {
        match chunk_result {
            Ok(chunk) => Ok(warp::hyper::body::Bytes::from(chunk)),
            Err(e) => {
                log::error!("Transcoding stream error: {}", e);
                // Convert to a hyper error
                Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            }
        }
    });

    // Create streaming response
    let body = warp::hyper::Body::wrap_stream(body_stream);
    let mut response = warp::reply::Response::new(body);

    // Set headers for streaming
    response.headers_mut().insert("content-type", get_transcoded_mime_type().parse().unwrap());
    response.headers_mut().insert("cache-control", "no-cache".parse().unwrap());
    response.headers_mut().insert("connection", "keep-alive".parse().unwrap());

    // Don't set content-length since we're streaming
    *response.status_mut() = StatusCode::OK;

    // CORS headers
    response.headers_mut().insert("access-control-allow-origin", "*".parse().unwrap());
    response.headers_mut().insert("access-control-allow-methods", "GET, HEAD, OPTIONS".parse().unwrap());
    response.headers_mut().insert("access-control-allow-headers", "range".parse().unwrap());

    Ok(response)
}

/// Handle native browser-supported video formats with range support
fn handle_native_video(
    filename: String,
    range_header: Option<String>,
    video_data: Vec<u8>,
) -> Result<warp::reply::Response, Rejection> {
    let total_size = video_data.len();

    // Parse range header if present
    let has_range = range_header.is_some();
    let (start, end) = if let Some(ref range) = range_header {
        parse_range_header(range, total_size)
    } else {
        (0, total_size - 1)
    };

    // Validate range
    if start >= total_size || end >= total_size || start > end {
        log::warn!("Invalid range request: {}-{} for file size {}", start, end, total_size);
        return Ok(warp::reply::with_status(
            "Invalid range",
            StatusCode::RANGE_NOT_SATISFIABLE,
        ).into_response());
    }

    let content_length = end - start + 1;
    let chunk = video_data[start..=end].to_vec();

    log::info!("Serving range {}-{}/{} ({} bytes) for {}", start, end, total_size, content_length, filename);

    // Build response with appropriate headers
    let mut response = warp::reply::Response::new(chunk.into());

    // Set content type based on file extension
    let content_type = get_content_type(&filename);
    response.headers_mut().insert("content-type", content_type.parse().unwrap());

    if has_range {
        // Partial content response
        response.headers_mut().insert("content-range",
            format!("bytes {}-{}/{}", start, end, total_size).parse().unwrap());
        response.headers_mut().insert("content-length", content_length.to_string().parse().unwrap());
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    } else {
        // Full content response
        response.headers_mut().insert("content-length", total_size.to_string().parse().unwrap());
        *response.status_mut() = StatusCode::OK;
    }

    // Enable range requests
    response.headers_mut().insert("accept-ranges", "bytes".parse().unwrap());

    // CORS headers for web access
    response.headers_mut().insert("access-control-allow-origin", "*".parse().unwrap());
    response.headers_mut().insert("access-control-allow-methods", "GET, HEAD, OPTIONS".parse().unwrap());
    response.headers_mut().insert("access-control-allow-headers", "range".parse().unwrap());

    Ok(response)
}

/// Get video data from MutAnt storage
async fn get_video_data(
    mutant: &MutAnt,
    user_key: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Attempting to get video data for key '{}'", user_key);
    
    // First try as a regular key
    match mutant.get(user_key, None, false).await {
        Ok(data) => {
            log::info!("Successfully retrieved video data as private key: {} ({} bytes)", user_key, data.len());
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
                                    let error_msg = format!("Failed to retrieve public video data for key '{}': {}", user_key, e);
                                    log::error!("{}", error_msg);
                                    Err(error_msg.into())
                                }
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse public address for key '{}': {}", user_key, e);
                            log::error!("{}", error_msg);
                            Err(error_msg.into())
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Failed to get public address for key '{}': {}", user_key, e);
                    log::error!("{}", error_msg);
                    log::info!("Key '{}' not found. Make sure the file is uploaded to MutAnt storage first.", user_key);
                    Err(error_msg.into())
                }
            }
        }
    }
}

/// Parse HTTP Range header
fn parse_range_header(range: &str, total_size: usize) -> (usize, usize) {
    // Range header format: "bytes=start-end" or "bytes=start-" or "bytes=-suffix"
    if let Some(bytes_range) = range.strip_prefix("bytes=") {
        if let Some((start_str, end_str)) = bytes_range.split_once('-') {
            let start = if start_str.is_empty() {
                // Suffix range: bytes=-500 (last 500 bytes)
                if let Ok(suffix) = end_str.parse::<usize>() {
                    total_size.saturating_sub(suffix)
                } else {
                    0
                }
            } else if let Ok(start) = start_str.parse::<usize>() {
                start
            } else {
                0
            };

            let end = if end_str.is_empty() {
                // Open range: bytes=500- (from 500 to end)
                total_size - 1
            } else if let Ok(end) = end_str.parse::<usize>() {
                end.min(total_size - 1)
            } else {
                total_size - 1
            };

            (start, end)
        } else {
            (0, total_size - 1)
        }
    } else {
        (0, total_size - 1)
    }
}

/// Get content type based on file extension
fn get_content_type(filename: &str) -> &'static str {
    let extension = filename.split('.').last().unwrap_or("").to_lowercase();
    match extension.as_str() {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "video/ogg",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "mkv" => "video/x-matroska",
        "m4v" => "video/x-m4v",
        "3gp" => "video/3gpp",
        _ => "application/octet-stream",
    }
}
