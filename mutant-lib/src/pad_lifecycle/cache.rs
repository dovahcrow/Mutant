use crate::index::persistence::{deserialize_index, serialize_index};
use crate::index::structure::MasterIndex;
use crate::network::NetworkChoice;
use crate::pad_lifecycle::error::PadLifecycleError;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Directory name within the user's data directory where cache files are stored.
const APP_DIR_NAME: &str = "mutant";
/// Filename for the index cache when using the Devnet.
const DEVNET_CACHE_FILE: &str = "index_cache.dev.cbor";
/// Filename for the index cache when using the Mainnet.
const MAINNET_CACHE_FILE: &str = "index_cache.main.cbor";

/// Determines the full path to the index cache file based on the network choice.
/// Ensures the necessary cache directory exists.
///
/// # Arguments
///
/// * `network_choice` - The network (Devnet or Mainnet) the cache is for.
///
/// # Errors
///
/// Returns `PadLifecycleError::CacheWriteError` if the user data directory
/// cannot be determined or if the cache directory cannot be created.
async fn get_cache_path(network_choice: NetworkChoice) -> Result<PathBuf, PadLifecycleError> {
    let base_cache_dir = dirs::data_dir().ok_or_else(|| {
        PadLifecycleError::CacheWriteError("Could not determine user data directory".to_string())
    })?;
    let cache_dir = base_cache_dir.join(APP_DIR_NAME);

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

/// Attempts to read and deserialize the `MasterIndex` from the local cache file.
///
/// # Arguments
///
/// * `network_choice` - The network choice to determine which cache file to read.
///
/// # Errors
///
/// Returns `PadLifecycleError::CacheReadError` if the cache path cannot be determined,
/// the file cannot be opened (other than NotFound), or reading fails.
///
/// # Returns
///
/// * `Ok(Some(MasterIndex))` if the cache exists and is successfully deserialized.
/// * `Ok(None)` if the cache file does not exist or fails to deserialize (treated as a cache miss).
/// * `Err(PadLifecycleError)` for other file I/O errors.
pub(crate) async fn read_cached_index(
    network_choice: NetworkChoice,
) -> Result<Option<MasterIndex>, PadLifecycleError> {
    let path = get_cache_path(network_choice).await?;
    trace!("Attempting to read cache file: {:?}", path);

    match fs::File::open(&path).await {
        Ok(mut file) => {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).await.map_err(|e| {
                PadLifecycleError::CacheReadError(format!(
                    "Failed to read cache file {:?}: {}",
                    path, e
                ))
            })?;

            if buffer.is_empty() {
                info!("Cache file {:?} is empty.", path);
                return Ok(None);
            }

            debug!("Read {} bytes from cache file {:?}", buffer.len(), path);
            match deserialize_index(&buffer) {
                Ok(index) => {
                    info!("Successfully deserialized index from cache {:?}", path);
                    Ok(Some(index))
                }
                Err(e) => {
                    error!("Failed to deserialize index from cache file {:?}: {}. Treating as cache miss.", path, e);

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

/// Serializes and writes the `MasterIndex` to the local cache file.
///
/// This overwrites the existing cache file if it exists.
///
/// # Arguments
///
/// * `index` - A reference to the `MasterIndex` to cache.
/// * `network_choice` - The network choice to determine which cache file to write.
///
/// # Errors
///
/// Returns `PadLifecycleError::CacheWriteError` if:
/// - The cache path cannot be determined.
/// - The index cannot be serialized.
/// - The cache file cannot be created/opened for writing.
/// - Writing to the file fails.
/// - Syncing the file to disk fails.
pub(crate) async fn write_cached_index(
    index: &MasterIndex,
    network_choice: NetworkChoice,
) -> Result<(), PadLifecycleError> {
    let path = get_cache_path(network_choice).await?;
    trace!("Attempting to write cache file: {:?}", path);

    let serialized_data = serialize_index(index).map_err(|e| {
        PadLifecycleError::CacheWriteError(format!("Failed to serialize index for caching: {}", e))
    })?;

    debug!(
        "Writing {} bytes to cache file {:?}",
        serialized_data.len(),
        path
    );

    let mut file = fs::File::create(&path).await.map_err(|e| {
        PadLifecycleError::CacheWriteError(format!(
            "Failed to create/open cache file {:?}: {}",
            path, e
        ))
    })?;

    file.write_all(&serialized_data).await.map_err(|e| {
        PadLifecycleError::CacheWriteError(format!(
            "Failed to write to cache file {:?}: {}",
            path, e
        ))
    })?;

    file.sync_all().await.map_err(|e| {
        PadLifecycleError::CacheWriteError(format!("Failed to sync cache file {:?}: {}", path, e))
    })?;

    info!("Successfully wrote index to cache file {:?}", path);
    Ok(())
}
