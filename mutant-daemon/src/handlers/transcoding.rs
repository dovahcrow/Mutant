use std::process::Stdio;
use tokio::io::AsyncRead;
use tokio::process::{Child, Command as TokioCommand};
use std::pin::Pin;
use std::task::{Context, Poll};
use futures_util::Stream;

/// Supported video formats that browsers can play natively
const BROWSER_SUPPORTED_FORMATS: &[&str] = &["mp4", "webm", "ogg"];

/// Video formats that need transcoding to MPEG-TS for WebSocket streaming
/// All formats except native MPEG-TS streams need transcoding for mpegts.js compatibility
const TRANSCODE_FORMATS: &[&str] = &["mp4", "m4v", "webm", "ogg", "mkv", "avi", "mov", "flv", "wmv", "3gp"];

/// Check if a video format needs transcoding based on file extension
pub fn needs_transcoding(filename: &str) -> bool {
    let extension = filename.split('.').last().unwrap_or("").to_lowercase();
    TRANSCODE_FORMATS.contains(&extension.as_str())
}

/// Get the appropriate MIME type for transcoded video
pub fn get_transcoded_mime_type() -> &'static str {
    "video/mp4" // We'll transcode to fragmented MP4 for streaming compatibility
}

/// Get the appropriate MIME type for MPEG-TS transcoded video
pub fn get_mpegts_mime_type() -> &'static str {
    "video/mp2t" // MPEG-TS format for WebSocket streaming
}

/// Streaming transcoder that converts video formats to MP4 on-the-fly
pub struct StreamingTranscoder {
    child: Child,
    stdout: tokio::process::ChildStdout,
}

impl StreamingTranscoder {
    /// Create a new streaming transcoder for the given video data
    pub async fn new(video_data: Vec<u8>, duration: Option<f64>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Starting FFmpeg transcoding process for {} bytes of video data", video_data.len());

        // Create FFmpeg command for transcoding to streamable format
        let mut cmd = TokioCommand::new("ffmpeg");
        let mut args = vec![
            "-i", "pipe:0",           // Input from stdin (auto-detect format)
            "-c:v", "libx264",        // Video codec: H.264
            "-c:a", "aac",            // Audio codec: AAC
            "-preset", "ultrafast",   // Fastest encoding for real-time
            "-tune", "zerolatency",   // Optimize for low latency
        ];

        // Use fragmented MP4 for streaming (this is the only viable option for pipes)
        args.extend_from_slice(&[
            "-movflags", "+frag_keyframe+empty_moov+default_base_moof",
            "-frag_duration", "1000000", // 1 second fragments
        ]);

        if duration.is_some() {
            log::info!("Using fragmented MP4 encoding with known duration for streaming");
        } else {
            log::info!("Using fragmented MP4 encoding without duration information");
        }

        args.extend_from_slice(&[
            "-f", "mp4",              // Output format: MP4
            "-y",                     // Overwrite output without asking
            "pipe:1"                  // Output to stdout
        ]);

        cmd.args(&args);

        cmd.stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped()); // Capture stderr for debugging

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn FFmpeg process: {}", e))?;

        // Write input data to FFmpeg stdin in a separate task
        if let Some(mut stdin) = child.stdin.take() {
            let data = video_data.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                log::debug!("Writing {} bytes to FFmpeg stdin", data.len());
                if let Err(e) = stdin.write_all(&data).await {
                    log::error!("Failed to write video data to FFmpeg stdin: {}", e);
                } else {
                    log::debug!("Successfully wrote all data to FFmpeg stdin");
                }
                if let Err(e) = stdin.shutdown().await {
                    log::error!("Failed to close FFmpeg stdin: {}", e);
                } else {
                    log::debug!("Successfully closed FFmpeg stdin");
                }
            });
        }

        // Capture stderr for debugging
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("FFmpeg stderr: {}", line);
                }
            });
        }

        let stdout = child.stdout.take()
            .ok_or("Failed to capture FFmpeg stdout")?;

        log::info!("FFmpeg transcoding process started successfully");

        Ok(StreamingTranscoder {
            child,
            stdout,
        })
    }
}

impl AsyncRead for StreamingTranscoder {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl Drop for StreamingTranscoder {
    fn drop(&mut self) {
        // Kill the FFmpeg process when the transcoder is dropped
        if let Err(e) = self.child.start_kill() {
            log::warn!("Failed to kill FFmpeg process: {}", e);
        }
    }
}

/// MPEG-TS streaming transcoder for WebSocket streaming
pub struct MpegTsTranscoder {
    child: Child,
    stdout: tokio::process::ChildStdout,
}

impl MpegTsTranscoder {
    /// Create a new MPEG-TS streaming transcoder for WebSocket streaming
    pub async fn new(video_data: Vec<u8>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Starting FFmpeg MPEG-TS transcoding process for {} bytes of video data", video_data.len());

        // Create FFmpeg command for transcoding to MPEG-TS format
        let mut cmd = TokioCommand::new("ffmpeg");
        let args = vec![
            "-i", "pipe:0",           // Input from stdin (auto-detect format)
            "-c:v", "libx264",        // Video codec: H.264 (force re-encoding)
            "-profile:v", "baseline", // H.264 baseline profile for maximum compatibility
            "-level", "3.0",          // H.264 level 3.0 for broad browser support
            "-pix_fmt", "yuv420p",    // Pixel format for maximum compatibility
            "-c:a", "aac",            // Audio codec: AAC
            "-ar", "44100",           // Audio sample rate
            "-ac", "2",               // Audio channels (stereo)
            "-preset", "ultrafast",   // Fastest encoding for real-time
            "-tune", "zerolatency",   // Optimize for low latency
            "-avoid_negative_ts", "make_zero", // Ensure timestamps start at zero
            "-fflags", "+genpts",     // Generate presentation timestamps
            "-f", "mpegts",           // Output format: MPEG-TS
            "-y",                     // Overwrite output without asking
            "pipe:1"                  // Output to stdout
        ];

        cmd.args(&args);

        cmd.stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped()); // Capture stderr for debugging

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn FFmpeg MPEG-TS process: {}", e))?;

        // Write input data to FFmpeg stdin in a separate task
        if let Some(mut stdin) = child.stdin.take() {
            let data = video_data.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                log::debug!("Writing {} bytes to FFmpeg MPEG-TS stdin", data.len());
                if let Err(e) = stdin.write_all(&data).await {
                    log::error!("Failed to write video data to FFmpeg MPEG-TS stdin: {}", e);
                } else {
                    log::debug!("Successfully wrote all data to FFmpeg MPEG-TS stdin");
                }
                if let Err(e) = stdin.shutdown().await {
                    log::error!("Failed to close FFmpeg MPEG-TS stdin: {}", e);
                } else {
                    log::debug!("Successfully closed FFmpeg MPEG-TS stdin");
                }
            });
        }

        // Capture stderr for debugging
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.contains("error") || line.contains("Error") || line.contains("failed") || line.contains("Failed") {
                        log::error!("FFmpeg MPEG-TS stderr: {}", line);
                    } else {
                        log::debug!("FFmpeg MPEG-TS stderr: {}", line);
                    }
                }
            });
        }

        let stdout = child.stdout.take()
            .ok_or("Failed to capture FFmpeg MPEG-TS stdout")?;

        log::info!("FFmpeg MPEG-TS transcoding process started successfully");

        Ok(MpegTsTranscoder {
            child,
            stdout,
        })
    }
}

impl AsyncRead for MpegTsTranscoder {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl Drop for MpegTsTranscoder {
    fn drop(&mut self) {
        // Kill the FFmpeg process when the transcoder is dropped
        if let Err(e) = self.child.start_kill() {
            log::warn!("Failed to kill FFmpeg MPEG-TS process: {}", e);
        }
    }
}

/// Stream transcoded video data in chunks
pub struct TranscodedVideoStream {
    transcoder: StreamingTranscoder,
    buffer: Vec<u8>,
}

impl TranscodedVideoStream {
    pub async fn new(video_data: Vec<u8>, duration: Option<f64>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let transcoder = StreamingTranscoder::new(video_data, duration).await?;
        Ok(TranscodedVideoStream {
            transcoder,
            buffer: vec![0; 8192], // 8KB buffer
        })
    }
}

impl Stream for TranscodedVideoStream {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Clear the buffer for fresh read
        this.buffer.clear();
        this.buffer.resize(8192, 0);

        let mut read_buf = tokio::io::ReadBuf::new(&mut this.buffer);

        match Pin::new(&mut this.transcoder).poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                let bytes_read = read_buf.filled().len();
                log::debug!("TranscodedVideoStream: Read {} bytes from FFmpeg", bytes_read);
                if bytes_read == 0 {
                    // End of stream
                    log::debug!("TranscodedVideoStream: End of stream reached");
                    Poll::Ready(None)
                } else {
                    let data = read_buf.filled().to_vec();
                    Poll::Ready(Some(Ok(data)))
                }
            }
            Poll::Ready(Err(e)) => {
                log::error!("TranscodedVideoStream: Read error: {}", e);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Probe video file to extract duration and other metadata
pub async fn probe_video_duration(video_data: &[u8]) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    log::debug!("Probing video duration for {} bytes of data", video_data.len());

    // Use ffprobe to get video duration
    let mut cmd = TokioCommand::new("ffprobe");
    cmd.args(&[
        "-v", "quiet",           // Suppress verbose output
        "-show_entries", "format=duration", // Only show duration
        "-of", "csv=p=0",        // Output as CSV without headers
        "-i", "pipe:0"           // Input from stdin
    ]);

    cmd.stdin(Stdio::piped())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    let mut child = cmd.spawn()
        .map_err(|e| format!("Failed to spawn ffprobe process: {}", e))?;

    // Write video data to ffprobe stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(video_data).await
            .map_err(|e| format!("Failed to write data to ffprobe: {}", e))?;
        stdin.shutdown().await
            .map_err(|e| format!("Failed to close ffprobe stdin: {}", e))?;
    }

    // Read the output
    let output = child.wait_with_output().await
        .map_err(|e| format!("Failed to read ffprobe output: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe failed: {}", stderr).into());
    }

    let duration_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let duration = duration_str.parse::<f64>()
        .map_err(|e| format!("Failed to parse duration '{}': {}", duration_str, e))?;

    log::info!("Probed video duration: {:.2} seconds", duration);
    Ok(duration)
}

/// Quick format detection using file magic bytes
pub fn detect_video_format(data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }

    // Check for common video format signatures
    if data.starts_with(b"\x00\x00\x00\x18ftypmp4") || data.starts_with(b"\x00\x00\x00\x20ftypmp4") {
        Some("mp4".to_string())
    } else if data.starts_with(b"\x1A\x45\xDF\xA3") {
        Some("mkv".to_string())
    } else if data.starts_with(b"RIFF") && data[8..12] == *b"AVI " {
        Some("avi".to_string())
    } else if data.starts_with(b"\x00\x00\x00\x14ftypqt") {
        Some("mov".to_string())
    } else if data.starts_with(b"FLV") {
        Some("flv".to_string())
    } else if data.starts_with(b"\x30\x26\xB2\x75\x8E\x66\xCF\x11") {
        Some("wmv".to_string())
    } else {
        // Fallback to extension-based detection
        None
    }
}
