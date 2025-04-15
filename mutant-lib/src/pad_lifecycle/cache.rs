use crate::index::persistence::{deserialize_index, serialize_index};
use crate::index::structure::MasterIndex;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::error::PadLifecycleError;
use log::{debug, error, info, trace};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CACHE_DIR: &str = ".mutant";
const DEVNET_CACHE_FILE: &str = "index_cache.dev.cbor";
const MAINNET_CACHE_FILE: &str = "index_cache.main.cbor";

/// Gets the expected path for the local index cache file based on network choice.
/// Ensures the cache directory exists.
async fn get_cache_path(network_choice: NetworkChoice) -> Result<PathBuf, PadLifecycleError> {
    // Use directories crate for a more robust home dir resolution? For now, assume relative path is ok.
    // Or require cache path in config?
    let cache_dir = PathBuf::from(CACHE_DIR);
    if !cache_dir.exists() {
        debug!("Cache directory {:?} does not exist, creating.", cache_dir);
        fs::create_dir_all(&cache_dir).await.map_err(|e| {
            PadLifecycleError::CacheWriteError(format!(
                "Failed to create cache directory {:?}: {}",
                cache_dir, e
            ))
        })?;
    }

    let filename = match network_choice {
        NetworkChoice::Devnet => DEVNET_CACHE_FILE,
        NetworkChoice::Mainnet => MAINNET_CACHE_FILE,
    };
    Ok(cache_dir.join(filename))
}

/// Reads and deserializes the MasterIndex from the local cache file.
/// Returns Ok(None) if the cache file does not exist.
pub(crate) async fn read_cached_index(
    network_choice: NetworkChoice,
) -> Result<Option<MasterIndex>, PadLifecycleError> {
    let path = get_cache_path(network_choice).await?;
    trace!("Attempting to read cache file: {:?}", path);

    match fs::File::open(&path).await {
        Ok(mut file) => {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).await.map_err(|e| {
                PadLifecycleError::CacheReadError(format!("Failed to read cache file {:?}: {}", path, e))
            })?;

            if buffer.is_empty() {
                info!("Cache file {:?} is empty.", path);
                return Ok(None); // Treat empty file same as non-existent
            }

            debug!("Read {} bytes from cache file {:?}", buffer.len(), path);
            match deserialize_index(&buffer) {
                Ok(index) => {
                    info!("Successfully deserialized index from cache {:?}", path);
                    Ok(Some(index))
                },
                Err(e) => {
                    error!("Failed to deserialize index from cache file {:?}: {}. Treating as cache miss.", path, e);
                    // Don't return error, just treat as cache miss
                    Ok(None)
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("Cache file {:?} not found.", path);
            Ok(None)
        }
        Err(e) => {
            error!("Failed to open cache file {:?}: {}", path, e);
            Err(PadLifecycleError::CacheReadError(format!(
                "Failed to open cache file {:?}: {}",
                path, e
            )))
        }
    }
}

/// Serializes and writes the MasterIndex to the local cache file.
pub(crate) async fn write_cached_index(
    index: &MasterIndex,
    network_choice: NetworkChoice,
) -> Result<(), PadLifecycleError> {
    let path = get_cache_path(network_choice).await?;
    trace!("Attempting to write cache file: {:?}", path);

    let serialized_data = serialize_index(index).map_err(|e| {
        // Map index serialization error to cache write error context
        PadLifecycleError::CacheWriteError(format!("Failed to serialize index for caching: {}", e))
    })?;

    debug!(
        "Writing {} bytes to cache file {:?}",
        serialized_data.len(),
        path
    );

    // Use temp file writing? For now, overwrite directly.
    let mut file = fs::File::create(&path).await.map_err(|e| {
        PadLifecycleError::CacheWriteError(format!("Failed to create/open cache file {:?}: {}", path, e))
    })?;

    file.write_all(&serialized_data).await.map_err(|e| {
        PadLifecycleError::CacheWriteError(format!("Failed to write to cache file {:?}: {}", path, e))
    })?;

    file.sync_all().await.map_err(|e| { // Ensure data is flushed
         PadLifecycleError::CacheWriteError(format!("Failed to sync cache file {:?}: {}", path, e))
    })?;


    info!("Successfully wrote index to cache file {:?}", path);
    Ok(())
}