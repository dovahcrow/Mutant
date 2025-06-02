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
    let totalSize = 0;
    let mediaSource = null;
    let sourceBuffer = null;
    let isSourceBufferReady = false;
    let pendingChunks = [];
    let hasStartedPlayback = false;
    let chunksProcessed = 0;
    let lastBufferCheck = 0;

    // Try MediaSource Extensions first for true progressive streaming
    console.log(`Attempting MediaSource Extensions for progressive MP4 playback`);
    initProgressivePlayer();



    function initProgressivePlayer() {
        if (!window.MediaSource) {
            console.error(`MediaSource not supported in this browser`);
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
                console.error(`No supported codec found for MP4 playback`);
                return;
            }

            isSourceBufferReady = true;
            console.log(`SourceBuffer ready with codec: ${codecsToTry[codecsToTry.findIndex(codec => MediaSource.isTypeSupported(codec))]}`);

            // Process any pending chunks that arrived before SourceBuffer was ready
            if (pendingChunks.length > 0) {
                console.log(`Processing ${pendingChunks.length} pending chunks`);
                const firstChunk = pendingChunks.shift();
                try {
                    sourceBuffer.appendBuffer(firstChunk);
                } catch (e) {
                    console.warn(`Failed to append first pending chunk:`, e);
                }
            }

            sourceBuffer.addEventListener('updateend', () => {
                chunksProcessed++;
                console.log(`SourceBuffer updateend event - buffered ranges: ${sourceBuffer.buffered.length}, chunks processed: ${chunksProcessed}`);

                // Check if MediaSource Extensions is working after processing several chunks
                if (chunksProcessed > 20 && sourceBuffer.buffered.length === 0) {
                    console.warn(`MediaSource Extensions not working - no buffered ranges after ${chunksProcessed} chunks. Falling back to direct video approach.`);
                    fallbackToDirectVideo();
                    return;
                }

                // Process next pending chunk
                if (pendingChunks.length > 0 && !sourceBuffer.updating) {
                    const nextChunk = pendingChunks.shift();
                    console.log(`Processing pending chunk of ${nextChunk.length} bytes`);
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
                } else if (sourceBuffer.buffered.length > 0) {
                    // Log buffering progress
                    const bufferedSeconds = sourceBuffer.buffered.end(sourceBuffer.buffered.length - 1);
                    console.log(`Buffer updated: ${bufferedSeconds.toFixed(2)} seconds buffered`);
                }
            });

            sourceBuffer.addEventListener('error', (e) => {
                console.error(`SourceBuffer error:`, e);
            });

            startWebSocket();
        });

        mediaSource.addEventListener('error', (e) => {
            console.error(`MediaSource error:`, e);
        });
    }

    function fallbackToDirectVideo() {
        console.log(`Falling back to direct video streaming approach`);

        // Clean up MediaSource Extensions
        if (mediaSource) {
            try {
                if (mediaSource.readyState === 'open') {
                    mediaSource.endOfStream();
                }
                URL.revokeObjectURL(videoElement.src);
            } catch (e) {
                // Ignore cleanup errors
            }
        }

        // Reset state
        isSourceBufferReady = false;
        sourceBuffer = null;
        mediaSource = null;
        pendingChunks = [];

        // Close WebSocket to stop receiving chunks
        if (websocket) {
            websocket.close();
        }

        // Convert WebSocket URL to HTTP streaming URL
        const httpUrl = websocketUrl.replace('ws://', 'http://').replace('/video_stream/', '/stream/');
        console.log(`Using direct HTTP streaming URL: ${httpUrl}`);

        // Set the video source directly to the HTTP streaming endpoint
        videoElement.src = httpUrl;

        // Clear loading indicator
        videoElement.style.background = '#000';
        videoElement.style.backgroundSize = '';
        videoElement.style.backgroundPosition = '';

        // Set up video element event listeners for direct streaming
        videoElement.addEventListener('loadstart', () => {
            console.log(`Direct video loadstart event for ${videoElementId}`);
        });

        videoElement.addEventListener('loadedmetadata', () => {
            console.log(`Direct video metadata loaded for ${videoElementId}, duration: ${videoElement.duration}`);
        });

        videoElement.addEventListener('canplay', () => {
            console.log(`Direct video can start playing for ${videoElementId}`);
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
                    console.log(`Direct video buffered: ${percent.toFixed(1)}% (${bufferedEnd.toFixed(1)}s / ${duration.toFixed(1)}s)`);
                }
            }
        });

        videoElement.addEventListener('error', (e) => {
            console.error(`Direct video error for ${videoElementId}:`, e);
            console.error('Direct video error details:', videoElement.error);
        });

        console.log(`Direct video streaming initialized for ${videoElementId}`);
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

                // Track total size for logging
                totalSize += chunk.length;

                // Log progress every 1MB or so
                if (totalSize % (1024 * 1024) < chunk.length) {
                    console.log(`Downloaded ${(totalSize / (1024 * 1024)).toFixed(1)}MB for ${videoElementId}`);
                }

                // Update video element with loading indicator on first chunk
                if (totalSize === chunk.length) {
                    videoElement.style.background = 'linear-gradient(45deg, #333 25%, transparent 25%), linear-gradient(-45deg, #333 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #333 75%), linear-gradient(-45deg, transparent 75%, #333 75%)';
                    videoElement.style.backgroundSize = '20px 20px';
                    videoElement.style.backgroundPosition = '0 0, 0 10px, 10px -10px, -10px 0px';
                }

                // Handle MediaSource Extensions progressive streaming
                if (isSourceBufferReady && sourceBuffer) {
                    try {
                        if (sourceBuffer.updating) {
                            // Queue the chunk if SourceBuffer is busy
                            console.log(`SourceBuffer busy, queuing chunk of ${chunk.length} bytes`);
                            pendingChunks.push(chunk);
                        } else {
                            // Append chunk immediately
                            console.log(`Appending chunk of ${chunk.length} bytes to SourceBuffer`);
                            sourceBuffer.appendBuffer(chunk);
                        }
                    } catch (e) {
                        console.warn(`Failed to append chunk to SourceBuffer:`, e);
                        // Add to pending chunks to retry later
                        pendingChunks.push(chunk);
                    }
                } else {
                    // Queue chunks until SourceBuffer is ready
                    console.log(`SourceBuffer not ready, queuing chunk of ${chunk.length} bytes`);
                    pendingChunks.push(chunk);
                }
            }
        };

        websocket.onclose = () => {
            console.log(`WebSocket closed for MP4 player: ${videoElementId}`);
            console.log(`Total size received: ${(totalSize / (1024 * 1024)).toFixed(1)}MB`);

            // Clear loading indicator
            videoElement.style.background = '#000';
            videoElement.style.backgroundSize = '';
            videoElement.style.backgroundPosition = '';

            // Handle end of stream for MediaSource Extensions
            if (sourceBuffer && mediaSource) {
                console.log(`Ending MediaSource stream - readyState: ${mediaSource.readyState}, updating: ${sourceBuffer.updating}`);
                try {
                    // Wait for any pending operations to complete
                    if (sourceBuffer.updating) {
                        sourceBuffer.addEventListener('updateend', () => {
                            try {
                                if (mediaSource.readyState === 'open') {
                                    mediaSource.endOfStream();
                                    console.log(`MediaSource stream ended for ${videoElementId}`);
                                } else {
                                    console.log(`MediaSource not open (${mediaSource.readyState}), cannot end stream`);
                                }
                            } catch (e) {
                                console.warn(`Error ending MediaSource stream in updateend:`, e);
                            }
                        }, { once: true });
                    } else {
                        if (mediaSource.readyState === 'open') {
                            mediaSource.endOfStream();
                            console.log(`MediaSource stream ended for ${videoElementId}`);
                        } else {
                            console.log(`MediaSource not open (${mediaSource.readyState}), cannot end stream`);
                        }
                    }
                } catch (e) {
                    console.warn(`Error ending MediaSource stream:`, e);
                }
            }
        };

        websocket.onerror = (error) => {
            console.error(`WebSocket error for MP4 player ${videoElementId}:`, error);
        };
    }



    window.mutantActiveVideoPlayers[videoElementId] = {
        videoElement: videoElement,
        websocket: websocket,
        mediaSource: mediaSource,
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
                        URL.revokeObjectURL(playerEntry.videoElement.src);
                    } catch (e) {
                        // Ignore cleanup errors
                    }
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
