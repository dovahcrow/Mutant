// mutant-web/src/utils/video_buffer.js
// Progressive video buffering utilities for MutAnt

window.mutantVideoBuffers = window.mutantVideoBuffers || {};

/**
 * Initialize a progressive video buffer
 * @param {string} videoId - Unique identifier for the video
 * @param {string} mimeType - MIME type of the video (e.g., 'video/mp4')
 * @returns {Object} - Result object with bufferId or error
 */
export async function initVideoBuffer(videoId, mimeType) {
    try {
        // Create a new MediaSource for progressive loading
        const mediaSource = new MediaSource();
        const objectUrl = URL.createObjectURL(mediaSource);
        
        const buffer = {
            mediaSource: mediaSource,
            objectUrl: objectUrl,
            sourceBuffer: null,
            mimeType: mimeType,
            chunks: [],
            totalSize: 0,
            loadedSize: 0,
            isReady: false,
            isEnded: false
        };
        
        // Store the buffer
        window.mutantVideoBuffers[videoId] = buffer;
        
        // Wait for MediaSource to be ready
        await new Promise((resolve, reject) => {
            mediaSource.addEventListener('sourceopen', () => {
                try {
                    // Create source buffer with the specified MIME type
                    buffer.sourceBuffer = mediaSource.addSourceBuffer(mimeType);
                    buffer.isReady = true;
                    resolve();
                } catch (err) {
                    reject(err);
                }
            });
            
            mediaSource.addEventListener('error', reject);
        });
        
        console.log(`initVideoBuffer: created buffer ${videoId} for ${mimeType}`);
        return { bufferId: videoId, objectUrl: objectUrl, error: null };
    } catch (err) {
        console.error("initVideoBuffer Error:", err);
        return { bufferId: null, objectUrl: null, error: err.message };
    }
}

/**
 * Append video data chunk to the buffer
 * @param {string} videoId - Video buffer identifier
 * @param {Uint8Array} chunk - Video data chunk
 * @param {boolean} isLast - Whether this is the last chunk
 * @returns {Object} - Result object with error status
 */
export async function appendVideoChunk(videoId, chunk, isLast = false) {
    const buffer = window.mutantVideoBuffers[videoId];
    if (!buffer) {
        console.error(`appendVideoChunk Error: Buffer not found for ID ${videoId}`);
        return { error: "Buffer not found" };
    }
    
    if (!buffer.isReady || !buffer.sourceBuffer) {
        console.error(`appendVideoChunk Error: Buffer not ready for ID ${videoId}`);
        return { error: "Buffer not ready" };
    }
    
    try {
        // Wait for source buffer to be ready for more data
        if (buffer.sourceBuffer.updating) {
            await new Promise(resolve => {
                buffer.sourceBuffer.addEventListener('updateend', resolve, { once: true });
            });
        }
        
        // Append the chunk
        buffer.sourceBuffer.appendBuffer(chunk);
        buffer.loadedSize += chunk.length;
        
        // Wait for the append to complete
        await new Promise(resolve => {
            buffer.sourceBuffer.addEventListener('updateend', resolve, { once: true });
        });
        
        // If this is the last chunk, signal end of stream
        if (isLast && !buffer.isEnded) {
            buffer.mediaSource.endOfStream();
            buffer.isEnded = true;
        }
        
        console.log(`appendVideoChunk: appended ${chunk.length} bytes to ${videoId}, total: ${buffer.loadedSize}`);
        return { error: null };
    } catch (err) {
        console.error(`appendVideoChunk Error for ID ${videoId}:`, err);
        return { error: err.message };
    }
}

/**
 * Get the current buffering progress
 * @param {string} videoId - Video buffer identifier
 * @returns {Object} - Progress information
 */
export function getVideoProgress(videoId) {
    const buffer = window.mutantVideoBuffers[videoId];
    if (!buffer) {
        return { error: "Buffer not found" };
    }
    
    return {
        loadedSize: buffer.loadedSize,
        totalSize: buffer.totalSize,
        progress: buffer.totalSize > 0 ? buffer.loadedSize / buffer.totalSize : 0,
        isReady: buffer.isReady,
        isEnded: buffer.isEnded,
        error: null
    };
}

/**
 * Set the total expected size for progress calculation
 * @param {string} videoId - Video buffer identifier
 * @param {number} totalSize - Total expected size in bytes
 */
export function setVideoTotalSize(videoId, totalSize) {
    const buffer = window.mutantVideoBuffers[videoId];
    if (buffer) {
        buffer.totalSize = totalSize;
        console.log(`setVideoTotalSize: set total size ${totalSize} for ${videoId}`);
    }
}

/**
 * Clean up video buffer resources
 * @param {string} videoId - Video buffer identifier
 */
export function cleanupVideoBuffer(videoId) {
    const buffer = window.mutantVideoBuffers[videoId];
    if (!buffer) {
        console.error(`cleanupVideoBuffer Error: Buffer not found for ID ${videoId}`);
        return { error: "Buffer not found" };
    }
    
    try {
        // Revoke the object URL to free memory
        if (buffer.objectUrl) {
            URL.revokeObjectURL(buffer.objectUrl);
        }
        
        // Clean up MediaSource
        if (buffer.mediaSource && buffer.mediaSource.readyState !== 'closed') {
            buffer.mediaSource.endOfStream();
        }
        
        // Remove from storage
        delete window.mutantVideoBuffers[videoId];
        
        console.log(`cleanupVideoBuffer: cleaned up buffer ${videoId}`);
        return { error: null };
    } catch (err) {
        console.error(`cleanupVideoBuffer Error for ID ${videoId}:`, err);
        return { error: err.message };
    }
}

/**
 * Clean up all video elements from the DOM that match a pattern
 * @param {string} keyPattern - Pattern to match video elements (e.g., file key)
 */
export function cleanupVideoElements(keyPattern) {
    try {
        // Find all video elements that might be related to this file
        const videos = document.querySelectorAll('video');
        const cleanedIds = [];

        console.log(`cleanupVideoElements: Looking for videos matching pattern: ${keyPattern}`);
        console.log(`cleanupVideoElements: Found ${videos.length} video elements in DOM`);

        videos.forEach(video => {
            const videoId = video.id;
            console.log(`cleanupVideoElements: Checking video element with ID: ${videoId}`);

            // Match both old format (video_key) and new format (video_hash)
            // Also clean up any video that doesn't have an ID (orphaned videos)
            const shouldCleanup = !videoId ||
                videoId.includes(keyPattern.replace('/', '_')) ||
                videoId.startsWith('video_');

            if (shouldCleanup) {
                console.log(`cleanupVideoElements: Cleaning up video element: ${videoId || 'no-id'}`);

                // Pause and clear the video
                try {
                    video.pause();
                    video.src = '';
                    video.load();

                    // Clear any source elements
                    const sources = video.querySelectorAll('source');
                    sources.forEach(source => {
                        source.src = '';
                        if (source.parentNode) {
                            source.parentNode.removeChild(source);
                        }
                    });
                } catch (videoErr) {
                    console.warn(`cleanupVideoElements: Error cleaning video ${videoId}:`, videoErr);
                }

                // Remove from DOM if it has a parent
                if (video.parentNode) {
                    video.parentNode.removeChild(video);
                }

                cleanedIds.push(videoId || 'no-id');
            }
        });

        if (cleanedIds.length > 0) {
            console.log(`cleanupVideoElements: cleaned up ${cleanedIds.length} video elements:`, cleanedIds);
        } else {
            console.log(`cleanupVideoElements: No video elements found to clean up for pattern: ${keyPattern}`);
        }

        return { error: null, cleanedCount: cleanedIds.length };
    } catch (err) {
        console.error("cleanupVideoElements Error:", err);
        return { error: err.message, cleanedCount: 0 };
    }
}

/**
 * Check if MediaSource API is supported
 * @returns {boolean} - Whether progressive video loading is supported
 */
export function isProgressiveVideoSupported() {
    return typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"');
}
