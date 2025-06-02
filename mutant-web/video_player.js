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
            return initMp4Player(videoElementId, websocketUrl, x, y, width, height);
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

    // Append to the document body
    document.body.appendChild(videoElement);

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

        // If we have a server duration and the video duration is not set or incorrect, use server duration
        const serverDuration = videoElement.dataset.serverDuration;
        if (serverDuration && (!videoElement.duration || videoElement.duration === Infinity || isNaN(videoElement.duration))) {
            const duration = parseFloat(serverDuration);
            console.log(`Using server duration ${duration}s instead of video duration ${videoElement.duration}s for ${videoElementId}`);
            // Note: We can't directly set videoElement.duration, but we can use it in our progress calculations
            videoElement.dataset.effectiveDuration = duration.toString();
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
    
    console.log(`HTTP streaming MP4 player initialized for ${videoElementId}`);
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
            } else if (playerEntry.type === 'mp4-http') {
                // Cleanup HTTP MP4 player
                if (playerEntry.videoElement) {
                    playerEntry.videoElement.pause();
                    playerEntry.videoElement.src = '';
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
