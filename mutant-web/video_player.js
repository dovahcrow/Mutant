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

    let hasStartedPlayback = false;
    let websocket = null;
    let chunks = [];
    let totalSize = 0;
    let currentBlobUrl = null;
    let updateInterval = null;

    // Use WebSocket-based progressive streaming for universal video format support
    console.log(`Using WebSocket-based progressive streaming for universal video playback`);
    initProgressiveWebSocketStreaming();



    function initProgressiveWebSocketStreaming() {
        // Start WebSocket connection
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

        console.log(`Progressive WebSocket streaming initialized for ${videoElementId}`);
    }

    function startWebSocket() {
        console.log(`Starting WebSocket connection for progressive streaming: ${websocketUrl}`);
        websocket = new WebSocket(websocketUrl);
        websocket.binaryType = 'arraybuffer';

        websocket.onopen = () => {
            console.log(`WebSocket connected for progressive streaming: ${videoElementId}`);
        };

        websocket.onmessage = (event) => {
            if (event.data instanceof ArrayBuffer) {
                const chunk = new Uint8Array(event.data);
                chunks.push(chunk);
                totalSize += chunk.length;

                // Log progress every 1MB
                if (totalSize % (1024 * 1024) < chunk.length) {
                    console.log(`Downloaded ${(totalSize / (1024 * 1024)).toFixed(1)}MB for ${videoElementId}`);
                }

                // Update loading indicator on first chunk
                if (chunks.length === 1) {
                    videoElement.style.background = 'linear-gradient(45deg, #333 25%, transparent 25%), linear-gradient(-45deg, #333 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #333 75%), linear-gradient(-45deg, transparent 75%, #333 75%)';
                    videoElement.style.backgroundSize = '20px 20px';
                    videoElement.style.backgroundPosition = '0 0, 0 10px, 10px -10px, -10px 0px';
                }

                // Start progressive updates after accumulating some data
                if (chunks.length === 20 && !hasStartedPlayback) { // ~5MB
                    startProgressiveUpdates();
                }
            }
        };

        websocket.onclose = () => {
            console.log(`WebSocket closed for progressive streaming: ${videoElementId}`);
            console.log(`Total size received: ${(totalSize / (1024 * 1024)).toFixed(1)}MB`);

            // Clear loading indicator
            videoElement.style.background = '#000';
            videoElement.style.backgroundSize = '';
            videoElement.style.backgroundPosition = '';

            // Stop progressive updates
            if (updateInterval) {
                clearInterval(updateInterval);
                updateInterval = null;
            }

            // Create final blob with all data
            createFinalBlob();
        };

        websocket.onerror = (error) => {
            console.error(`WebSocket error for progressive streaming ${videoElementId}:`, error);
        };
    }
    function startProgressiveUpdates() {
        console.log(`Starting progressive updates for ${videoElementId}`);

        // Create initial blob and start playback
        updateVideoBlob();

        // Set up interval to update the blob periodically as more data arrives
        updateInterval = setInterval(() => {
            updateVideoBlob();
        }, 2000); // Update every 2 seconds
    }

    function updateVideoBlob() {
        // Create blob from all accumulated chunks
        const combinedData = new Uint8Array(totalSize);
        let offset = 0;

        for (const chunk of chunks) {
            combinedData.set(chunk, offset);
            offset += chunk.length;
        }

        // Create new blob
        const blob = new Blob([combinedData], { type: 'video/mp4' });
        const newBlobUrl = URL.createObjectURL(blob);

        // Clean up previous blob URL
        if (currentBlobUrl) {
            URL.revokeObjectURL(currentBlobUrl);
        }

        currentBlobUrl = newBlobUrl;

        // Update video source if this is the first time or if video has ended/errored
        if (!hasStartedPlayback || videoElement.ended || videoElement.error) {
            console.log(`Updating video source with ${(totalSize / (1024 * 1024)).toFixed(1)}MB of data`);

            // Store current time to resume from same position
            const currentTime = videoElement.currentTime || 0;

            // Update source
            videoElement.src = newBlobUrl;

            // Resume from previous position
            if (currentTime > 0) {
                videoElement.currentTime = currentTime;
            }

            // Start playback if not already started
            if (!hasStartedPlayback) {
                hasStartedPlayback = true;
                videoElement.play().catch(e => {
                    console.log(`Autoplay prevented for ${videoElementId}, user will need to click play:`, e);
                });
            }
        }
    }

    function createFinalBlob() {
        console.log(`Creating final blob with all ${chunks.length} chunks (${(totalSize / (1024 * 1024)).toFixed(1)}MB)`);

        // Create final blob with all data
        updateVideoBlob();

        console.log(`Final video ready for ${videoElementId}`);
    }





    window.mutantActiveVideoPlayers[videoElementId] = {
        videoElement: videoElement,
        websocket: websocket,
        currentBlobUrl: currentBlobUrl,
        updateInterval: updateInterval,
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
                if (playerEntry.updateInterval) {
                    clearInterval(playerEntry.updateInterval);
                }
                if (playerEntry.currentBlobUrl) {
                    URL.revokeObjectURL(playerEntry.currentBlobUrl);
                }
                try {
                    URL.revokeObjectURL(playerEntry.videoElement.src);
                } catch (e) {
                    // Ignore cleanup errors
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
