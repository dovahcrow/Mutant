// Global storage for active video players
if (!window.mutantActiveVideoPlayers) {
    window.mutantActiveVideoPlayers = {};
}

// Detect video format from filename
function getVideoFormat(filename) {
    const extension = filename.split('.').pop().toLowerCase();

    // Native browser-supported formats
    switch (extension) {
        case 'mp4':
        case 'm4v':
        case 'webm':
        case 'ogg':
            return 'mp4'; // Use native HTML5 video player
        case 'ts':
        case 'mts':
        case 'm2ts':
            return 'mpegts'; // Use mpegts.js for transport streams
        case 'flv':
            return 'flv'; // Use mpegts.js for FLV
        // All other formats (mkv, avi, mov, wmv, etc.) will be transcoded to MP4 by the server
        case 'mkv':
        case 'avi':
        case 'mov':
        case 'wmv':
        case '3gp':
        default:
            return 'mp4'; // Server will transcode to MP4, use native player
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
            // Use WebSocket streaming for MP4 with MediaSource Extensions
            return initWebSocketMp4Player(videoElementId, websocketUrl, x, y, width, height);
        case 'mpegts':
        case 'flv':
            return initMpegtsPlayer(videoElementId, websocketUrl, x, y, width, height);
        default:
            console.error(`Unsupported video format: ${format} for file: ${filename}`);
            return;
    }
}

// MPEG-TS player using mpegts.js
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
        cleanupMpegtsPlayer(videoElementId);
    }

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
    videoElement.style.zIndex = '1000';

    // Append to the document body
    document.body.appendChild(videoElement);

    const player = mpegts.createPlayer({
        type: 'mpegts',
        isLive: false,
        url: websocketUrl,
    });

    player.attachMediaElement(videoElement);

    player.on(mpegts.Events.ERROR, (err) => {
        console.error(`mpegts.js Error for ${videoElementId}:`, err);
    });

    player.on(mpegts.Events.LOADING_COMPLETE, () => {
        console.log(`mpegts.js: Loading complete for ${videoElementId}`);
    });

    try {
        player.load();
    } catch (e) {
        console.error(`Error calling player.load() for ${videoElementId}:`, e);
        cleanupMpegtsPlayer(videoElementId);
        return;
    }

    window.mutantActiveVideoPlayers[videoElementId] = {
        player: player,
        videoElement: videoElement,
        type: 'mpegts'
    };
    console.log(`MPEG-TS player initialized and stored for ${videoElementId}`);
}

// WebSocket MP4 player using MediaSource Extensions for true streaming
function initWebSocketMp4Player(videoElementId, websocketUrl, x, y, width, height) {
    console.log(`initWebSocketMp4Player called for element: ${videoElementId}, url: ${websocketUrl}`);
    console.log(`Positioning - x: ${x}, y: ${y}, width: ${width}, height: ${height}`);

    // Check if MediaSource is supported
    if (!window.MediaSource) {
        console.error('MediaSource Extensions not supported in this browser.');
        // Fallback to HTTP streaming
        return initMp4Player(videoElementId, websocketUrl, x, y, width, height);
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

    // Style the video element for absolute positioning
    videoElement.style.position = 'absolute';
    videoElement.style.left = x + 'px';
    videoElement.style.top = y + 'px';
    videoElement.style.width = width + 'px';
    videoElement.style.height = height + 'px';
    videoElement.style.backgroundColor = '#000';
    videoElement.style.border = '2px solid #00ff00'; // Green border for WebSocket MP4
    videoElement.style.zIndex = '1000';

    // Append to the document body
    document.body.appendChild(videoElement);

    // Create MediaSource
    const mediaSource = new MediaSource();
    let sourceBuffer = null;
    let websocket = null;
    let isSourceOpen = false;

    // Set up MediaSource
    videoElement.src = URL.createObjectURL(mediaSource);

    mediaSource.addEventListener('sourceopen', () => {
        console.log(`MediaSource opened for ${videoElementId}`);
        isSourceOpen = true;

        try {
            // Create source buffer for MP4
            sourceBuffer = mediaSource.addSourceBuffer('video/mp4; codecs="avc1.42E01E, mp4a.40.2"');

            sourceBuffer.addEventListener('updateend', () => {
                console.log(`SourceBuffer update completed for ${videoElementId}`);
            });

            sourceBuffer.addEventListener('error', (e) => {
                console.error(`SourceBuffer error for ${videoElementId}:`, e);
            });

            // Start WebSocket connection
            startWebSocketConnection();

        } catch (e) {
            console.error(`Failed to create SourceBuffer for ${videoElementId}:`, e);
        }
    });

    function startWebSocketConnection() {
        console.log(`Starting WebSocket connection to: ${websocketUrl}`);
        websocket = new WebSocket(websocketUrl);
        websocket.binaryType = 'arraybuffer';

        websocket.onopen = () => {
            console.log(`WebSocket connected for ${videoElementId}`);
        };

        websocket.onmessage = (event) => {
            if (event.data instanceof ArrayBuffer && sourceBuffer && !sourceBuffer.updating) {
                try {
                    console.log(`Received ${event.data.byteLength} bytes for ${videoElementId}`);
                    sourceBuffer.appendBuffer(event.data);

                    // Try to start playback when we have enough data
                    if (videoElement.readyState >= 2 && videoElement.paused) {
                        videoElement.play().catch(e => {
                            console.log(`Autoplay prevented for ${videoElementId}:`, e);
                        });
                    }
                } catch (e) {
                    console.error(`Failed to append buffer for ${videoElementId}:`, e);
                }
            }
        };

        websocket.onclose = () => {
            console.log(`WebSocket closed for ${videoElementId}`);
            // End the stream
            if (mediaSource.readyState === 'open') {
                try {
                    mediaSource.endOfStream();
                } catch (e) {
                    console.error(`Failed to end stream for ${videoElementId}:`, e);
                }
            }
        };

        websocket.onerror = (error) => {
            console.error(`WebSocket error for ${videoElementId}:`, error);
        };
    }

    // Set up video event listeners
    videoElement.addEventListener('loadstart', () => {
        console.log(`Video load started for ${videoElementId}`);
    });

    videoElement.addEventListener('loadedmetadata', () => {
        console.log(`Video metadata loaded for ${videoElementId}. Duration: ${videoElement.duration}s`);
    });

    videoElement.addEventListener('canplay', () => {
        console.log(`Video can start playing for ${videoElementId}`);
    });

    videoElement.addEventListener('playing', () => {
        console.log(`Video started playing for ${videoElementId}`);
    });

    videoElement.addEventListener('error', (e) => {
        console.error(`Video error for ${videoElementId}:`, e);
        console.error('Video error details:', videoElement.error);
    });

    // Store the player info for cleanup
    window.mutantActiveVideoPlayers[videoElementId] = {
        videoElement: videoElement,
        mediaSource: mediaSource,
        websocket: websocket,
        type: 'websocket-mp4'
    };

    console.log(`WebSocket MP4 player initialized for ${videoElementId}`);
}

// MP4 player using HTTP streaming for true progressive playback
function initMp4Player(videoElementId, websocketUrl, x, y, width, height) {
    console.log(`initMp4Player called for element: ${videoElementId}, url: ${websocketUrl}`);
    console.log(`Positioning - x: ${x}, y: ${y}, width: ${width}, height: ${height}`);

    // Create the video element
    let videoElement = document.createElement('video');
    videoElement.id = videoElementId;
    videoElement.setAttribute('controls', 'true');
    videoElement.setAttribute('preload', 'metadata');

    // Style the video element for absolute positioning
    videoElement.style.position = 'absolute';
    videoElement.style.left = x + 'px';
    videoElement.style.top = y + 'px';
    videoElement.style.width = width + 'px';
    videoElement.style.height = height + 'px';
    videoElement.style.backgroundColor = '#000';
    videoElement.style.border = '2px solid #ff8c00';
    videoElement.style.zIndex = '1000';

    // Create a custom duration overlay for transcoded videos
    const durationOverlay = document.createElement('div');
    durationOverlay.id = `duration_overlay_${videoElementId}`;
    durationOverlay.style.position = 'absolute';
    durationOverlay.style.right = '10px';
    durationOverlay.style.bottom = '40px';
    durationOverlay.style.backgroundColor = 'rgba(0, 0, 0, 0.7)';
    durationOverlay.style.color = 'white';
    durationOverlay.style.padding = '4px 8px';
    durationOverlay.style.borderRadius = '4px';
    durationOverlay.style.fontSize = '14px';
    durationOverlay.style.fontFamily = 'monospace';
    durationOverlay.style.zIndex = '1001';
    durationOverlay.style.pointerEvents = 'none';
    durationOverlay.style.display = 'none'; // Hidden initially
    durationOverlay.dataset.shouldShow = 'false'; // Track if overlay should be visible

    // Append to the document body
    document.body.appendChild(videoElement);
    document.body.appendChild(durationOverlay);

    console.log(`Video element created and positioned at (${x}, ${y}) with size ${width}x${height}`);

    // Convert WebSocket URL to HTTP URL for direct streaming
    const httpUrl = websocketUrl.replace('ws://', 'http://').replace('/video_stream/', '/video/');
    console.log(`Using HTTP streaming URL: ${httpUrl}`);

    // Set the video source directly - this enables true HTTP streaming with range requests
    videoElement.src = httpUrl;

    // Try to get duration from server headers for transcoded videos
    fetch(httpUrl, { method: 'HEAD' })
        .then(response => {
            const durationHeader = response.headers.get('x-video-duration');
            if (durationHeader) {
                const duration = parseFloat(durationHeader);
                console.log(`Got duration from server header: ${duration} seconds for ${videoElementId}`);
                // Store the duration for use when metadata loads
                videoElement.dataset.serverDuration = duration.toString();

                // Show the custom duration overlay for transcoded content
                durationOverlay.dataset.shouldShow = 'true';
                durationOverlay.style.display = 'block';

                // Function to format seconds to MM:SS
                function formatTime(seconds) {
                    const mins = Math.floor(seconds / 60);
                    const secs = Math.floor(seconds % 60);
                    return `${mins}:${secs.toString().padStart(2, '0')}`;
                }

                // Function to update the overlay
                function updateDurationOverlay() {
                    const currentTime = videoElement.currentTime || 0;
                    const totalTime = duration;
                    durationOverlay.textContent = `${formatTime(currentTime)} / ${formatTime(totalTime)}`;
                }

                // Update overlay immediately and set up interval
                updateDurationOverlay();
                const overlayUpdateInterval = setInterval(updateDurationOverlay, 100);

                // Store the interval for cleanup
                videoElement.dataset.overlayUpdateInterval = overlayUpdateInterval;

                console.log(`Custom duration overlay enabled for transcoded video: ${duration}s for ${videoElementId}`);
            }
        })
        .catch(e => {
            console.log(`Failed to fetch duration header for ${videoElementId}:`, e);
        });



    // Set up video event listeners for HTTP streaming
    videoElement.addEventListener('loadstart', () => {
        console.log(`Video load started for ${videoElementId}`);
    });

    videoElement.addEventListener('loadedmetadata', () => {
        console.log(`Video metadata loaded for ${videoElementId}. Duration: ${videoElement.duration}s`);

        // If we have a server duration, always use it for transcoded content
        const serverDuration = videoElement.dataset.serverDuration;
        if (serverDuration) {
            const duration = parseFloat(serverDuration);
            console.log(`Using server duration ${duration}s for transcoded content ${videoElementId}`);
            videoElement.dataset.effectiveDuration = duration.toString();

            // Override the duration property to return our server duration
            Object.defineProperty(videoElement, 'duration', {
                get: function() {
                    return duration;
                },
                configurable: true
            });

            console.log(`Duration override applied: ${videoElement.duration}s for ${videoElementId}`);
        }
    });

    videoElement.addEventListener('loadeddata', () => {
        console.log(`Video data loaded for ${videoElementId}`);
    });

    videoElement.addEventListener('canplay', () => {
        console.log(`Video can start playing for ${videoElementId}`);
        // Auto-play when ready (if allowed by browser)
        videoElement.play().catch(e => {
            console.log(`Autoplay prevented for ${videoElementId}, user will need to click play:`, e);
        });
    });

    videoElement.addEventListener('playing', () => {
        console.log(`Video started playing for ${videoElementId}`);
    });

    videoElement.addEventListener('waiting', () => {
        console.log(`Video is buffering for ${videoElementId}`);
    });

    videoElement.addEventListener('progress', () => {
        if (videoElement.buffered.length > 0) {
            const bufferedEnd = videoElement.buffered.end(videoElement.buffered.length - 1);
            const effectiveDuration = videoElement.dataset.effectiveDuration || videoElement.dataset.serverDuration;
            const duration = effectiveDuration ? parseFloat(effectiveDuration) : videoElement.duration;

            if (duration > 0) {
                const percent = (bufferedEnd / duration * 100).toFixed(1);
                console.log(`Video buffered: ${percent}% (${bufferedEnd.toFixed(1)}s / ${duration.toFixed(1)}s) for ${videoElementId}`);
            }
        }
    });

    videoElement.addEventListener('error', (e) => {
        console.error(`Video error for ${videoElementId}:`, e);
        console.error('Video error details:', videoElement.error);
    });

    // Store the player info for cleanup
    window.mutantActiveVideoPlayers[videoElementId] = {
        videoElement: videoElement,
        type: 'mp4-http'
    };

    // Set up a periodic check to maintain duration override for transcoded content
    const durationCheckInterval = setInterval(() => {
        const serverDuration = videoElement.dataset.serverDuration;
        if (serverDuration && videoElement.duration !== parseFloat(serverDuration)) {
            const duration = parseFloat(serverDuration);
            console.log(`Re-applying duration override: ${duration}s for ${videoElementId}`);
            Object.defineProperty(videoElement, 'duration', {
                get: function() {
                    return duration;
                },
                configurable: true
            });
        }
    }, 1000); // Check every second

    // Store the interval for cleanup
    window.mutantActiveVideoPlayers[videoElementId].durationCheckInterval = durationCheckInterval;

    console.log(`HTTP streaming MP4 player initialized for ${videoElementId}`);
}

// Function to update video player position and visibility
function updateVideoPlayerPosition(videoElementId, x, y, width, height, isVisible = true) {
    console.log(`updateVideoPlayerPosition called for element: ${videoElementId}, x: ${x}, y: ${y}, width: ${width}, height: ${height}, visible: ${isVisible}`);
    const playerEntry = window.mutantActiveVideoPlayers[videoElementId];

    if (playerEntry && playerEntry.videoElement) {
        playerEntry.videoElement.style.left = x + 'px';
        playerEntry.videoElement.style.top = y + 'px';
        playerEntry.videoElement.style.width = width + 'px';
        playerEntry.videoElement.style.height = height + 'px';

        // Control visibility and z-index based on tab state
        if (isVisible) {
            playerEntry.videoElement.style.display = 'block';
            playerEntry.videoElement.style.zIndex = '1000';
        } else {
            playerEntry.videoElement.style.display = 'none';
            playerEntry.videoElement.style.zIndex = '-1';
        }

        // Also handle duration overlay if it exists
        const overlayId = `duration_overlay_${videoElementId}`;
        const overlay = document.getElementById(overlayId);
        if (overlay) {
            if (isVisible) {
                overlay.style.display = overlay.dataset.shouldShow === 'true' ? 'block' : 'none';
                overlay.style.zIndex = '1001';
            } else {
                overlay.style.display = 'none';
                overlay.style.zIndex = '-1';
            }
        }

        console.log(`Video player ${videoElementId} position and visibility updated.`);
    } else {
        console.warn(`No active player found for ID ${videoElementId} to update position.`);
    }
}

// Function to hide video player (for when tab becomes inactive)
function hideVideoPlayer(videoElementId) {
    console.log(`hideVideoPlayer called for element: ${videoElementId}`);
    const playerEntry = window.mutantActiveVideoPlayers[videoElementId];

    if (playerEntry && playerEntry.videoElement) {
        playerEntry.videoElement.style.display = 'none';
        playerEntry.videoElement.style.zIndex = '-1';

        // Also hide duration overlay
        const overlayId = `duration_overlay_${videoElementId}`;
        const overlay = document.getElementById(overlayId);
        if (overlay) {
            overlay.style.display = 'none';
            overlay.style.zIndex = '-1';
        }

        console.log(`Video player ${videoElementId} hidden.`);
    } else {
        console.warn(`No active player found for ID ${videoElementId} to hide.`);
    }
}

// Function to show video player (for when tab becomes active)
function showVideoPlayer(videoElementId) {
    console.log(`showVideoPlayer called for element: ${videoElementId}`);
    const playerEntry = window.mutantActiveVideoPlayers[videoElementId];

    if (playerEntry && playerEntry.videoElement) {
        playerEntry.videoElement.style.display = 'block';
        playerEntry.videoElement.style.zIndex = '1000';

        // Also show duration overlay if it should be visible
        const overlayId = `duration_overlay_${videoElementId}`;
        const overlay = document.getElementById(overlayId);
        if (overlay && overlay.dataset.shouldShow === 'true') {
            overlay.style.display = 'block';
            overlay.style.zIndex = '1001';
        }

        console.log(`Video player ${videoElementId} shown.`);
    } else {
        console.warn(`No active player found for ID ${videoElementId} to show.`);
    }
}

// Function to update z-index for all video players based on visibility
function updateVideoPlayersZIndex(visiblePlayerIds) {
    console.log(`updateVideoPlayersZIndex called with visible IDs: ${visiblePlayerIds}`);

    // Parse the comma-separated list of visible player IDs
    const visibleIds = visiblePlayerIds ? visiblePlayerIds.split(',').filter(id => id.trim() !== '') : [];
    const visibleIdSet = new Set(visibleIds);

    // Iterate through all video players and set appropriate z-index
    for (const videoElementId in window.mutantActiveVideoPlayers) {
        const playerEntry = window.mutantActiveVideoPlayers[videoElementId];
        if (playerEntry && playerEntry.videoElement) {
            if (visibleIdSet.has(videoElementId)) {
                // Visible tab - high z-index
                playerEntry.videoElement.style.zIndex = '1000';
                playerEntry.videoElement.style.display = 'block';

                // Also update overlay z-index
                const overlayId = `duration_overlay_${videoElementId}`;
                const overlay = document.getElementById(overlayId);
                if (overlay && overlay.dataset.shouldShow === 'true') {
                    overlay.style.zIndex = '1001';
                    overlay.style.display = 'block';
                }
            } else {
                // Hidden tab - low z-index (behind UI elements)
                playerEntry.videoElement.style.zIndex = '-1';

                // Also hide overlay
                const overlayId = `duration_overlay_${videoElementId}`;
                const overlay = document.getElementById(overlayId);
                if (overlay) {
                    overlay.style.zIndex = '-1';
                }
            }
        }
    }

    console.log(`Updated z-index for ${Object.keys(window.mutantActiveVideoPlayers).length} video players, ${visibleIds.length} visible`);
}

// Function to hide all inactive video players
function hideInactiveVideoPlayers(activePlayerIds) {
    console.log(`hideInactiveVideoPlayers called with active IDs: ${activePlayerIds}`);

    // Parse the comma-separated list of active player IDs
    const activeIds = activePlayerIds ? activePlayerIds.split(',').filter(id => id.trim() !== '') : [];
    const activeIdSet = new Set(activeIds);

    // Iterate through all video players and hide those not in the active set
    for (const videoElementId in window.mutantActiveVideoPlayers) {
        if (!activeIdSet.has(videoElementId)) {
            hideVideoPlayer(videoElementId);
        } else {
            // Ensure active players are visible
            showVideoPlayer(videoElementId);
        }
    }

    console.log(`Processed ${Object.keys(window.mutantActiveVideoPlayers).length} video players, ${activeIds.length} active`);
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
            } else if (playerEntry.type === 'websocket-mp4') {
                // Cleanup WebSocket MP4 player
                if (playerEntry.videoElement) {
                    playerEntry.videoElement.pause();
                    playerEntry.videoElement.src = '';
                }
                // Close WebSocket connection
                if (playerEntry.websocket) {
                    playerEntry.websocket.close();
                }
                // Clean up MediaSource
                if (playerEntry.mediaSource && playerEntry.mediaSource.readyState === 'open') {
                    try {
                        playerEntry.mediaSource.endOfStream();
                    } catch (e) {
                        console.warn(`Failed to end MediaSource for ${videoElementId}:`, e);
                    }
                }
                console.log(`WebSocket MP4 player for ${videoElementId} destroyed.`);
            } else if (playerEntry.type === 'mp4-http') {
                // Cleanup HTTP MP4 player
                if (playerEntry.videoElement) {
                    playerEntry.videoElement.pause();
                    playerEntry.videoElement.src = '';
                }
                // Clear duration check interval
                if (playerEntry.durationCheckInterval) {
                    clearInterval(playerEntry.durationCheckInterval);
                }
                // Clear overlay update interval
                if (playerEntry.videoElement && playerEntry.videoElement.dataset.overlayUpdateInterval) {
                    clearInterval(parseInt(playerEntry.videoElement.dataset.overlayUpdateInterval));
                }
                // Remove duration overlay
                const overlayId = `duration_overlay_${videoElementId}`;
                const overlay = document.getElementById(overlayId);
                if (overlay && overlay.parentNode) {
                    overlay.parentNode.removeChild(overlay);
                }
                console.log(`HTTP MP4 player for ${videoElementId} destroyed.`);
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
