console.log("video_player.js loaded and executing");

// mutant-web/video_player.js

window.mutantActiveVideoPlayers = window.mutantActiveVideoPlayers || {};

// Detect video format from filename
function getVideoFormat(filename) {
    const ext = filename.toLowerCase().split('.').pop();
    switch (ext) {
        case 'mp4':
        case 'm4v':
        case 'mov':
            return 'mp4';
        case 'ts':
        case 'mts':
        case 'm2ts':
            return 'mpegts';
        case 'flv':
            return 'flv';
        default:
            return 'unknown';
    }
}

// Main function to initialize appropriate video player based on format
function initVideoPlayer(videoElementId, websocketUrl, x, y, width, height) {
    console.log(`initVideoPlayer called for element: ${videoElementId}, url: ${websocketUrl}`);
    console.log(`Positioning - x: ${x}, y: ${y}, width: ${width}, height: ${height}`);

    // Extract filename from WebSocket URL to detect format
    const urlParts = websocketUrl.split('/');
    const filename = urlParts[urlParts.length - 1];
    const format = getVideoFormat(filename);

    console.log(`Detected video format: ${format} for file: ${filename}`);

    // If a player for this ID already exists, clean it up first
    if (window.mutantActiveVideoPlayers[videoElementId]) {
        console.warn(`Player for ${videoElementId} already exists. Cleaning up old one.`);
        cleanupVideoPlayer(videoElementId);
    }

    // Choose appropriate player based on format
    switch (format) {
        case 'mp4':
            return initMp4Player(videoElementId, websocketUrl, x, y, width, height);
        case 'mpegts':
        case 'flv':
            return initMpegtsPlayer(videoElementId, websocketUrl, x, y, width, height);
        default:
            console.error(`Unsupported video format: ${format} for file: ${filename}`);
            return;
    }
}

// Legacy function name for backward compatibility
function initMpegtsPlayer(videoElementId, websocketUrl, x, y, width, height) {
    console.log(`initMpegtsPlayer called for element: ${videoElementId}, url: ${websocketUrl}`);
    console.log(`Positioning - x: ${x}, y: ${y}, width: ${width}, height: ${height}`);

    if (typeof mpegts === 'undefined') {
        console.error('mpegts.js is not loaded.');
        return;
    }

    if (!mpegts.isSupported()) {
        console.error('MPEG-TS playback is not supported in this browser.');
        return;
    }

    // If a player for this ID already exists, clean it up first
    if (window.mutantActiveVideoPlayers[videoElementId]) {
        console.warn(`Player for ${videoElementId} already exists. Cleaning up old one.`);
        cleanupVideoPlayer(videoElementId);
    }

    // Create the video element
    let videoElement = document.createElement('video');
    videoElement.id = videoElementId;
    videoElement.setAttribute('controls', 'true');
    // videoElement.setAttribute('autoplay', 'true'); // Autoplay can be aggressive, enable if desired

    // Style the video element for absolute positioning
    videoElement.style.position = 'absolute';
    videoElement.style.left = x + 'px';
    videoElement.style.top = y + 'px';
    videoElement.style.width = width + 'px';
    videoElement.style.height = height + 'px';
    videoElement.style.backgroundColor = '#000'; // Optional: background color while loading

    // Append to the document body. Egui manages the space, this floats on top.
    document.body.appendChild(videoElement);

    const player = mpegts.createPlayer({
        type: 'mpegts', // Explicitly set for MPEG-TS streams.
        isLive: false, // Set to true if it's a live stream
        url: websocketUrl,
        // Other configurations:
        // enableWorker: true, // Use worker for transmuxing if available and beneficial
        // lazyLoad: false, // Start loading immediately
        // autoCleanupSourceBuffer: true,
    });

    player.attachMediaElement(videoElement);

    player.on(mpegts.Events.ERROR, (err) => {
        console.error(`mpegts.js Error for ${videoElementId}:`, err);
        // Potentially cleanup and remove video element on critical error
        // cleanupMpegtsPlayer(videoElementId);
    });

    player.on(mpegts.Events.LOADING_COMPLETE, () => {
        console.log(`mpegts.js: Loading complete for ${videoElementId}`);
    });

    player.on(mpegts.Events.RECOVERED_EARLY_EOF, () => {
        console.log(`mpegts.js: Recovered from early EOF for ${videoElementId}`);
    });

    player.on(mpegts.Events.METADATA_ARRIVED, (metadata) => {
        console.log(`mpegts.js: Metadata arrived for ${videoElementId}`, metadata);
    });

    player.on(mpegts.Events.STATISTICS_INFO, (stats) => {
        // console.log(`mpegts.js: Stats for ${videoElementId}`, stats);
    });


    try {
        player.load();
        // player.play(); // Autoplay if desired, or let user click controls
    } catch (e) {
        console.error(`Error calling player.load() or player.play() for ${videoElementId}:`, e);
        cleanupMpegtsPlayer(videoElementId); // Clean up if load fails immediately
        return;
    }

    window.mutantActiveVideoPlayers[videoElementId] = {
        player: player,
        videoElement: videoElement,
        type: 'mpegts'
    };
    console.log(`MPEG-TS player initialized and stored for ${videoElementId}`);
}

// MP4 player using progressive streaming with MediaSource Extensions
function initMp4Player(videoElementId, websocketUrl, x, y, width, height) {
    console.log(`initMp4Player called for element: ${videoElementId}, url: ${websocketUrl}`);
    console.log(`Positioning - x: ${x}, y: ${y}, width: ${width}, height: ${height}`);

    // Create the video element
    let videoElement = document.createElement('video');
    videoElement.id = videoElementId;
    videoElement.setAttribute('controls', 'true');

    // Style the video element for absolute positioning
    videoElement.style.position = 'absolute';
    videoElement.style.left = x + 'px';
    videoElement.style.top = y + 'px';
    videoElement.style.width = width + 'px';
    videoElement.style.height = height + 'px';
    videoElement.style.backgroundColor = '#000';
    videoElement.style.border = '2px solid #ff8c00'; // Orange border for debugging
    videoElement.style.zIndex = '1000'; // Ensure it's on top

    // Append to the document body
    document.body.appendChild(videoElement);

    console.log(`Video element created and positioned at (${x}, ${y}) with size ${width}x${height}`);
    console.log(`Video element in DOM:`, document.getElementById(videoElementId));

    let websocket = null;
    let chunks = [];
    let totalSize = 0;
    let mediaSource = null;
    let sourceBuffer = null;
    let isSourceBufferReady = false;
    let pendingChunks = [];
    let hasStartedPlayback = false;

    // Use single-blob approach for MP4 files to avoid flickering
    console.log(`Using single-blob approach for MP4 playback`);
    initSingleBlobPlayer();

    function initSingleBlobPlayer() {
        // Start WebSocket immediately
        startWebSocket();

        // Set up video element event listeners
        videoElement.addEventListener('loadstart', () => {
            console.log(`Video loadstart event for ${videoElementId}`);
        });

        videoElement.addEventListener('loadedmetadata', () => {
            console.log(`Video metadata loaded for ${videoElementId}, duration: ${videoElement.duration}`);
        });

        videoElement.addEventListener('canplay', () => {
            console.log(`Video can start playing for ${videoElementId}`);
            if (!hasStartedPlayback) {
                hasStartedPlayback = true;
                // Try to start playback automatically
                videoElement.play().catch(e => {
                    console.log(`Autoplay prevented for ${videoElementId}, user will need to click play:`, e);
                });
            }
        });

        videoElement.addEventListener('waiting', () => {
            console.log(`Video is waiting for more data for ${videoElementId}`);
        });

        videoElement.addEventListener('playing', () => {
            console.log(`Video started playing for ${videoElementId}`);
        });

        videoElement.addEventListener('error', (e) => {
            console.error(`Video error for ${videoElementId}:`, e);
            console.error('Video error details:', videoElement.error);
        });

        console.log(`Single blob player initialized for ${videoElementId}`);
    }

    function initHttpStreamingPlayer() {
        // Convert WebSocket URL to HTTP streaming URL
        const httpUrl = websocketUrl.replace('ws://', 'http://').replace('/video_stream/', '/stream/');
        console.log(`Using HTTP streaming URL: ${httpUrl}`);

        // Set the video source directly to the HTTP streaming endpoint
        videoElement.src = httpUrl;

        // Set up video element event listeners
        videoElement.addEventListener('loadstart', () => {
            console.log(`Video loadstart event for ${videoElementId}`);
        });

        videoElement.addEventListener('loadedmetadata', () => {
            console.log(`Video metadata loaded for ${videoElementId}, duration: ${videoElement.duration}`);
        });

        videoElement.addEventListener('canplay', () => {
            console.log(`Video can start playing for ${videoElementId}`);
            if (!hasStartedPlayback) {
                hasStartedPlayback = true;
                // Try to start playback automatically
                videoElement.play().catch(e => {
                    console.log(`Autoplay prevented for ${videoElementId}, user will need to click play:`, e);
                });
            }
        });

        videoElement.addEventListener('waiting', () => {
            console.log(`Video is waiting for more data for ${videoElementId}`);
        });

        videoElement.addEventListener('playing', () => {
            console.log(`Video started playing for ${videoElementId}`);
        });

        videoElement.addEventListener('progress', () => {
            if (videoElement.buffered.length > 0) {
                const bufferedEnd = videoElement.buffered.end(videoElement.buffered.length - 1);
                const duration = videoElement.duration;
                if (duration > 0) {
                    const percent = (bufferedEnd / duration) * 100;
                    console.log(`Video buffered: ${percent.toFixed(1)}% (${bufferedEnd.toFixed(1)}s / ${duration.toFixed(1)}s)`);
                }
            }
        });

        videoElement.addEventListener('error', (e) => {
            console.error(`Video error for ${videoElementId}:`, e);
            console.error('Video error details:', videoElement.error);
            // Fallback to WebSocket approach
            console.log(`HTTP streaming failed, falling back to WebSocket approach`);
            initProgressiveBlobPlayer();
        });

        console.log(`HTTP streaming player initialized for ${videoElementId}`);
    }

    function initProgressiveBlobPlayer() {
        // Start WebSocket immediately
        startWebSocket();

        // Set up video element for progressive playback
        videoElement.addEventListener('loadstart', () => {
            console.log(`Video loadstart event for ${videoElementId}`);
        });

        videoElement.addEventListener('loadedmetadata', () => {
            console.log(`Video metadata loaded for ${videoElementId}, duration: ${videoElement.duration}`);
        });

        videoElement.addEventListener('canplay', () => {
            console.log(`Video can start playing for ${videoElementId}`);
            if (!hasStartedPlayback) {
                hasStartedPlayback = true;
                // Try to start playback automatically
                videoElement.play().catch(e => {
                    console.log(`Autoplay prevented for ${videoElementId}, user will need to click play:`, e);
                });
            }
        });

        videoElement.addEventListener('waiting', () => {
            console.log(`Video is waiting for more data for ${videoElementId}`);
        });

        videoElement.addEventListener('playing', () => {
            console.log(`Video started playing for ${videoElementId}`);
        });

        videoElement.addEventListener('error', (e) => {
            console.error(`Video error for ${videoElementId}:`, e);
            console.error('Video error details:', videoElement.error);
        });
    }

    function initProgressivePlayer() {
        if (!window.MediaSource) {
            console.warn(`MediaSource not supported, falling back to blob approach`);
            initBlobPlayer();
            return;
        }

        mediaSource = new MediaSource();
        const objectUrl = URL.createObjectURL(mediaSource);
        videoElement.src = objectUrl;

        mediaSource.addEventListener('sourceopen', () => {
            console.log(`MediaSource opened for progressive streaming`);

            // Try different codec combinations for MP4
            const codecsToTry = [
                'video/mp4; codecs="avc1.42E01E, mp4a.40.2"',
                'video/mp4; codecs="avc1.64001E, mp4a.40.2"',
                'video/mp4; codecs="avc1.4D401E, mp4a.40.2"',
                'video/mp4; codecs="avc1.42001E, mp4a.40.2"',
                'video/mp4'
            ];

            let sourceBufferCreated = false;

            for (const codec of codecsToTry) {
                try {
                    if (MediaSource.isTypeSupported(codec)) {
                        console.log(`Using codec: ${codec}`);
                        sourceBuffer = mediaSource.addSourceBuffer(codec);
                        sourceBufferCreated = true;
                        break;
                    }
                } catch (e) {
                    console.warn(`Failed to create SourceBuffer with codec ${codec}:`, e);
                }
            }

            if (!sourceBufferCreated) {
                console.warn(`No supported codec found, falling back to blob approach`);
                fallbackToBlobPlayer();
                return;
            }

            isSourceBufferReady = true;

            sourceBuffer.addEventListener('updateend', () => {
                // Process next pending chunk
                if (pendingChunks.length > 0 && !sourceBuffer.updating) {
                    const nextChunk = pendingChunks.shift();
                    try {
                        sourceBuffer.appendBuffer(nextChunk);
                    } catch (e) {
                        console.warn(`Failed to append pending chunk:`, e);
                        // Continue with remaining chunks
                    }
                }

                // Start playback once we have some data buffered
                if (!hasStartedPlayback && sourceBuffer.buffered.length > 0) {
                    hasStartedPlayback = true;
                    const bufferedSeconds = sourceBuffer.buffered.end(0);
                    console.log(`Starting progressive playback with ${bufferedSeconds.toFixed(2)} seconds buffered`);

                    // Try to start playback
                    videoElement.currentTime = 0;
                    videoElement.play().catch(e => {
                        console.log(`Autoplay prevented, user will need to click play:`, e);
                    });
                }
            });

            sourceBuffer.addEventListener('error', (e) => {
                console.warn(`SourceBuffer error:`, e);
                // Don't fallback immediately, continue trying
            });

            startWebSocket();
        });

        mediaSource.addEventListener('error', (e) => {
            console.warn(`MediaSource error, falling back to blob approach:`, e);
            fallbackToBlobPlayer();
        });
    }

    function fallbackToBlobPlayer() {
        console.log(`Falling back to blob-based progressive download`);
        if (mediaSource) {
            try {
                URL.revokeObjectURL(videoElement.src);
            } catch (e) {
                // Ignore cleanup errors
            }
        }
        initBlobPlayer();
    }

    function initMediaSourcePlayer() {
        mediaSource = new MediaSource();
        const objectUrl = URL.createObjectURL(mediaSource);
        videoElement.src = objectUrl;

        mediaSource.addEventListener('sourceopen', () => {
            console.log(`MediaSource opened for progressive streaming`);
            try {
                sourceBuffer = mediaSource.addSourceBuffer('video/mp4; codecs="avc1.42E01E, mp4a.40.2"');
                isSourceBufferReady = true;

                sourceBuffer.addEventListener('updateend', () => {
                    // Process next pending chunk
                    if (pendingChunks.length > 0 && !sourceBuffer.updating) {
                        const nextChunk = pendingChunks.shift();
                        try {
                            sourceBuffer.appendBuffer(nextChunk);
                        } catch (e) {
                            console.warn(`Failed to append chunk, falling back to blob approach:`, e);
                            fallbackToBlobPlayer();
                        }
                    }

                    // Start playback once we have some data buffered
                    if (!hasStartedPlayback && sourceBuffer.buffered.length > 0) {
                        hasStartedPlayback = true;
                        console.log(`Starting progressive playback with ${sourceBuffer.buffered.end(0)} seconds buffered`);
                    }
                });

                sourceBuffer.addEventListener('error', (e) => {
                    console.warn(`SourceBuffer error, falling back to blob approach:`, e);
                    fallbackToBlobPlayer();
                });

                startWebSocket();
            } catch (e) {
                console.warn(`Failed to create SourceBuffer, falling back to blob approach:`, e);
                fallbackToBlobPlayer();
            }
        });

        mediaSource.addEventListener('error', (e) => {
            console.warn(`MediaSource error, falling back to blob approach:`, e);
            fallbackToBlobPlayer();
        });
    }

    function fallbackToBlobPlayer() {
        console.log(`Falling back to blob-based player`);
        if (mediaSource) {
            try {
                if (mediaSource.readyState === 'open') {
                    mediaSource.endOfStream();
                }
            } catch (e) {
                // Ignore errors during cleanup
            }
        }
        isSourceBufferReady = false;
        sourceBuffer = null;
        mediaSource = null;

        // Reset video element source
        videoElement.src = '';

        // If we already have chunks, create blob immediately
        if (chunks.length > 0) {
            createBlobVideo();
        }
    }

    function initBlobPlayer() {
        // Legacy function - redirect to progressive blob player
        initProgressiveBlobPlayer();
    }

    function startWebSocket() {
        console.log(`Starting WebSocket connection for MP4: ${websocketUrl}`);
        websocket = new WebSocket(websocketUrl);
        websocket.binaryType = 'arraybuffer';

        websocket.onopen = () => {
            console.log(`WebSocket connected for MP4 player: ${videoElementId}`);
        };

        websocket.onmessage = (event) => {
            if (event.data instanceof ArrayBuffer) {
                const chunk = new Uint8Array(event.data);

                // Always store the chunk
                chunks.push(chunk);
                totalSize += chunk.length;

                // Log progress every 10MB or so
                if (chunks.length % 40 === 0) {
                    console.log(`Downloaded ${totalSize} bytes (${chunks.length} chunks) for ${videoElementId}`);
                }

                // Update video element with loading indicator on first chunk
                if (chunks.length === 1) {
                    videoElement.style.background = 'linear-gradient(45deg, #333 25%, transparent 25%), linear-gradient(-45deg, #333 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #333 75%), linear-gradient(-45deg, transparent 75%, #333 75%)';
                    videoElement.style.backgroundSize = '20px 20px';
                    videoElement.style.backgroundPosition = '0 0, 0 10px, 10px -10px, -10px 0px';
                }

                // Create the blob once we have a reasonable buffer (about 10MB)
                // This should be enough for the video to start playing and have some buffer
                if (chunks.length === 40 && !hasStartedPlayback) {
                    createInitialBlob();
                }
            }
        };

        websocket.onclose = () => {
            console.log(`WebSocket closed for MP4 player: ${videoElementId}`);
            console.log(`Total chunks received: ${chunks.length}, total size: ${totalSize} bytes`);

            // Final blob update with all chunks
            createFinalBlob();
        };

        websocket.onerror = (error) => {
            console.error(`WebSocket error for MP4 player ${videoElementId}:`, error);
        };
    }

    function createInitialBlob() {
        // Create initial blob with current chunks to start playback
        if (chunks.length === 0 || hasStartedPlayback) return;

        console.log(`Creating initial blob with ${chunks.length} chunks for early playback`);

        // Combine all chunks into a single Uint8Array
        const combinedData = new Uint8Array(totalSize);
        let offset = 0;

        for (const chunk of chunks) {
            combinedData.set(chunk, offset);
            offset += chunk.length;
        }

        // Create a blob from the combined data
        const blob = new Blob([combinedData], { type: 'video/mp4' });
        const blobUrl = URL.createObjectURL(blob);

        // Set the video source to the blob URL (only once!)
        videoElement.src = blobUrl;
        hasStartedPlayback = true;

        // Store the blob URL for cleanup
        if (window.mutantActiveVideoPlayers[videoElementId]) {
            window.mutantActiveVideoPlayers[videoElementId].blobUrl = blobUrl;
        }

        console.log(`Initial blob created with ${totalSize} bytes, video should start playing soon`);
    }

    function updateProgressiveBlob() {
        // Legacy function - no longer used to avoid flickering
        console.log(`updateProgressiveBlob called but skipped to avoid flickering`);
    }

    function updateBlobVideo() {
        // Legacy function - redirect to progressive version
        updateProgressiveBlob();
    }

    function createFinalBlob() {
        console.log(`Creating final blob video from ${chunks.length} chunks`);

        // Clear loading indicator
        videoElement.style.background = '#000';
        videoElement.style.backgroundSize = '';
        videoElement.style.backgroundPosition = '';

        // Final update with all chunks
        updateProgressiveBlob();

        console.log(`Final video ready for ${videoElementId}`);
    }

    function createBlobVideo() {
        // Legacy function - redirect to final blob
        createFinalBlob();
    }

    window.mutantActiveVideoPlayers[videoElementId] = {
        videoElement: videoElement,
        websocket: websocket,
        mediaSource: mediaSource,
        blobUrl: null, // Will be set when video is ready
        type: 'mp4'
    };
    console.log(`MP4 player initialized and stored for ${videoElementId}`);
}

// Universal cleanup function for all player types
function cleanupVideoPlayer(videoElementId) {
    console.log(`cleanupVideoPlayer called for element: ${videoElementId}`);
    const playerEntry = window.mutantActiveVideoPlayers[videoElementId];

    if (playerEntry) {
        try {
            if (playerEntry.type === 'mpegts' && playerEntry.player) {
                // Cleanup mpegts.js player
                playerEntry.player.pause();
                playerEntry.player.unload();
                playerEntry.player.detachMediaElement();
                playerEntry.player.destroy();
                console.log(`MPEG-TS player for ${videoElementId} destroyed.`);
            } else if (playerEntry.type === 'mp4') {
                // Cleanup MP4 player
                if (playerEntry.websocket) {
                    playerEntry.websocket.close();
                }
                if (playerEntry.mediaSource) {
                    try {
                        if (playerEntry.mediaSource.readyState === 'open') {
                            playerEntry.mediaSource.endOfStream();
                        }
                    } catch (e) {
                        // Ignore cleanup errors
                    }
                }
                if (playerEntry.blobUrl) {
                    URL.revokeObjectURL(playerEntry.blobUrl);
                }
                console.log(`MP4 player for ${videoElementId} destroyed.`);
            }
        } catch (e) {
            console.error(`Error during player cleanup for ${videoElementId}:`, e);
        }

        // Remove video element from DOM
        if (playerEntry.videoElement && playerEntry.videoElement.parentNode) {
            playerEntry.videoElement.parentNode.removeChild(playerEntry.videoElement);
            console.log(`Video element ${videoElementId} removed from DOM.`);
        }

        delete window.mutantActiveVideoPlayers[videoElementId];
        console.log(`Player entry for ${videoElementId} removed.`);
    } else {
        console.warn(`No active player found for ID ${videoElementId} to cleanup.`);
        // Fallback: try to remove element from DOM if it exists
        const element = document.getElementById(videoElementId);
        if (element && element.parentNode) {
            element.parentNode.removeChild(element);
            console.log(`Fallback: Video element ${videoElementId} removed from DOM.`);
        }
    }
}

// Legacy function for backward compatibility
function cleanupMpegtsPlayer(videoElementId) {
    cleanupVideoPlayer(videoElementId);
}

// Make functions available for wasm_bindgen (Rust)
// These are already global in this script context, so wasm_bindgen can find them by name.
// No explicit export needed if using `#[wasm_bindgen(js_name = ...)]` on Rust side
// and the JS functions are global.
// Explicitly attaching to window object makes them definitively global.
