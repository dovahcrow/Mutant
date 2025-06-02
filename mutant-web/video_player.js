// Global storage for active video players
if (!window.mutantActiveVideoPlayers) {
    window.mutantActiveVideoPlayers = {};
}

// Detect video format from filename
function getVideoFormat(filename) {
    const extension = filename.split('.').pop().toLowerCase();

    // Format detection for unified WebSocket streaming
    switch (extension) {
        case 'mp4':
        case 'm4v':
        case 'webm':
        case 'ogg':
        case 'avi':
        case 'mov':
        case 'wmv':
        case '3gp':
            return 'mpegts'; // These formats use WebSocket streaming with transcoding
        case 'mkv':
        case 'ts':
        case 'mts':
        case 'm2ts':
        case 'flv':
            return 'mpegts'; // These formats use direct WebSocket streaming (no transcoding)
        default:
            return 'mpegts'; // Default to WebSocket streaming
    }
}

// Main function to initialize appropriate video player based on format
function initVideoPlayer(videoElementId, websocketUrl, x, y, width, height) {
    
    

    // Extract filename from WebSocket URL to detect format
    const urlParts = websocketUrl.split('/');
    const filename = urlParts[urlParts.length - 1];
    const format = getVideoFormat(filename);

    

    // If a player for this ID already exists, clean it up first
    if (window.mutantActiveVideoPlayers[videoElementId]) {
        console.warn(`Player for ${videoElementId} already exists. Cleaning up old one.`);
        cleanupVideoPlayer(videoElementId);
    }

    // All formats now use MPEG-TS player via WebSocket streaming
    switch (format) {
        case 'mpegts':
            return initMpegtsPlayer(videoElementId, websocketUrl, x, y, width, height);
        default:
            console.error(`Unsupported video format: ${format} for file: ${filename}`);
            return;
    }
}

// MPEG-TS player using mpegts.js
function initMpegtsPlayer(videoElementId, websocketUrl, x, y, width, height) {
    
    

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
    
}

// MP4 player using HTTP streaming for true progressive playback
function initMp4Player(videoElementId, websocketUrl, x, y, width, height) {
    
    

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

    

    // Convert WebSocket URL to HTTP URL for direct streaming
    const httpUrl = websocketUrl.replace('ws://', 'http://').replace('/video_stream/', '/video/');
    

    // Set the video source directly - this enables true HTTP streaming with range requests
    videoElement.src = httpUrl;

    // Try to get duration from server headers for transcoded videos
    fetch(httpUrl, { method: 'HEAD' })
        .then(response => {
            const durationHeader = response.headers.get('x-video-duration');
            if (durationHeader) {
                const duration = parseFloat(durationHeader);
                
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

                
            }
        })
        .catch(e => {
            
        });



    // Set up video event listeners for HTTP streaming
    videoElement.addEventListener('loadstart', () => {
        
    });

    videoElement.addEventListener('loadedmetadata', () => {
        

        // If we have a server duration, always use it for transcoded content
        const serverDuration = videoElement.dataset.serverDuration;
        if (serverDuration) {
            const duration = parseFloat(serverDuration);
            
            videoElement.dataset.effectiveDuration = duration.toString();

            // Override the duration property to return our server duration
            Object.defineProperty(videoElement, 'duration', {
                get: function() {
                    return duration;
                },
                configurable: true
            });

            
        }
    });

    videoElement.addEventListener('loadeddata', () => {
        
    });

    videoElement.addEventListener('canplay', () => {
        
        // Auto-play when ready (if allowed by browser)
        videoElement.play().catch(e => {
            
        });
    });

    videoElement.addEventListener('playing', () => {
        
    });

    videoElement.addEventListener('waiting', () => {
        
    });

    videoElement.addEventListener('progress', () => {
        if (videoElement.buffered.length > 0) {
            const bufferedEnd = videoElement.buffered.end(videoElement.buffered.length - 1);
            const effectiveDuration = videoElement.dataset.effectiveDuration || videoElement.dataset.serverDuration;
            const duration = effectiveDuration ? parseFloat(effectiveDuration) : videoElement.duration;

            if (duration > 0) {
                const percent = (bufferedEnd / duration * 100).toFixed(1);
                
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

    
}

// Function to update video player position and visibility
function updateVideoPlayerPosition(videoElementId, x, y, width, height, isVisible = true) {
    
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

        
    } else {
        console.warn(`No active player found for ID ${videoElementId} to update position.`);
    }
}

// Function to hide video player (for when tab becomes inactive)
function hideVideoPlayer(videoElementId) {
    
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

        
    } else {
        console.warn(`No active player found for ID ${videoElementId} to hide.`);
    }
}

// Function to show video player (for when tab becomes active)
function showVideoPlayer(videoElementId) {
    
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

        
    } else {
        console.warn(`No active player found for ID ${videoElementId} to show.`);
    }
}

// Function to update z-index for all video players based on visibility
function updateVideoPlayersZIndex(visiblePlayerIds) {
    

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

    
}

// Function to hide all inactive video players
function hideInactiveVideoPlayers(activePlayerIds) {
    

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

    
}

// Universal cleanup function for all player types
function cleanupVideoPlayer(videoElementId) {
    
    const playerEntry = window.mutantActiveVideoPlayers[videoElementId];

    if (playerEntry) {
        try {
            if (playerEntry.type === 'mpegts' && playerEntry.player) {
                // Cleanup mpegts.js player
                playerEntry.player.pause();
                playerEntry.player.unload();
                playerEntry.player.detachMediaElement();
                playerEntry.player.destroy();
                
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
                
            }
        } catch (e) {
            console.error(`Error during player cleanup for ${videoElementId}:`, e);
        }

        // Remove video element from DOM
        if (playerEntry.videoElement && playerEntry.videoElement.parentNode) {
            playerEntry.videoElement.parentNode.removeChild(playerEntry.videoElement);
            
        }

        delete window.mutantActiveVideoPlayers[videoElementId];
        
    } else {
        console.warn(`No active player found for ID ${videoElementId} to cleanup.`);
        // Fallback: try to remove element from DOM if it exists
        const element = document.getElementById(videoElementId);
        if (element && element.parentNode) {
            element.parentNode.removeChild(element);
            
        }
    }
}

// Legacy function for backward compatibility
function cleanupMpegtsPlayer(videoElementId) {
    cleanupVideoPlayer(videoElementId);
}
