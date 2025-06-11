use wasm_bindgen::prelude::*;
use js_sys::Uint8Array;

#[wasm_bindgen(module = "/src/utils/video_buffer.js")]
extern "C" {
    #[wasm_bindgen(js_name = initVideoBuffer, catch)]
    pub async fn js_init_video_buffer(video_id: &str, mime_type: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = appendVideoChunk, catch)]
    pub async fn js_append_video_chunk(video_id: &str, chunk: Uint8Array, is_last: bool) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = getVideoProgress)]
    pub fn js_get_video_progress(video_id: &str) -> JsValue;

    #[wasm_bindgen(js_name = cleanupVideoBuffer, catch)]
    pub fn js_cleanup_video_buffer(video_id: &str) -> Result<JsValue, JsValue>;
}

/// Initialize a video buffer for streaming playback
pub async fn init_video_buffer(video_id: &str, mime_type: &str) -> Result<String, String> {
    match js_init_video_buffer(video_id, mime_type).await {
        Ok(result) => {
            // Try to get the objectUrl property from the result
            let object_url = js_sys::Reflect::get(&result, &"objectUrl".into())
                .map_err(|_| "Failed to get objectUrl property")?;

            if object_url.is_null() || object_url.is_undefined() {
                // Check for error
                let error = js_sys::Reflect::get(&result, &"error".into())
                    .map_err(|_| "Failed to get error property")?;

                if !error.is_null() && !error.is_undefined() {
                    return Err(error.as_string().unwrap_or("Unknown error".to_string()));
                }

                return Err("No object URL returned".to_string());
            }

            object_url.as_string().ok_or("Object URL is not a string".to_string())
        }
        Err(e) => Err(format!("Failed to initialize video buffer: {:?}", e))
    }
}

/// Append a chunk of video data to the buffer
pub async fn append_video_chunk(video_id: &str, chunk: &[u8], is_last: bool) -> Result<(), String> {
    let uint8_array = Uint8Array::new_with_length(chunk.len() as u32);
    uint8_array.copy_from(chunk);

    match js_append_video_chunk(video_id, uint8_array, is_last).await {
        Ok(result) => {
            // Check for error
            let error = js_sys::Reflect::get(&result, &"error".into())
                .map_err(|_| "Failed to get error property")?;

            if !error.is_null() && !error.is_undefined() {
                return Err(error.as_string().unwrap_or("Unknown error".to_string()));
            }

            Ok(())
        }
        Err(e) => Err(format!("Failed to append video chunk: {:?}", e))
    }
}

/// Get video buffering progress
pub fn get_video_progress(video_id: &str) -> Result<VideoProgress, String> {
    let result = js_get_video_progress(video_id);

    // Check for error
    let error = js_sys::Reflect::get(&result, &"error".into())
        .map_err(|_| "Failed to get error property")?;

    if !error.is_null() && !error.is_undefined() {
        return Err(error.as_string().unwrap_or("Unknown error".to_string()));
    }

    let loaded_size = js_sys::Reflect::get(&result, &"loadedSize".into())
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u64;

    let total_size = js_sys::Reflect::get(&result, &"totalSize".into())
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u64;

    let progress = js_sys::Reflect::get(&result, &"progress".into())
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;

    let is_ready = js_sys::Reflect::get(&result, &"isReady".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let is_ended = js_sys::Reflect::get(&result, &"isEnded".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(VideoProgress {
        loaded_size,
        total_size,
        progress,
        is_ready,
        is_ended,
    })
}

/// Clean up a video buffer
pub fn cleanup_video_buffer(video_id: &str) -> Result<(), String> {
    match js_cleanup_video_buffer(video_id) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Failed to cleanup video buffer: {:?}", e))
    }
}

#[derive(Debug, Clone)]
pub struct VideoProgress {
    pub loaded_size: u64,
    pub total_size: u64,
    pub progress: f32,
    pub is_ready: bool,
    pub is_ended: bool,
}
