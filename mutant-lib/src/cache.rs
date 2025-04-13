use crate::error::Error;
use crate::mutant::data_structures::MasterIndexStorage;
use crate::mutant::NetworkChoice;
use dirs;
use log::{debug, error, info};
use serde_cbor;
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs::{self, File};
use tokio::io::{AsyncWriteExt, BufWriter};

const CACHE_BASE_DIR_NAME: &str = "mutant";
const CACHE_FILE_NAME: &str = "index.cbor";

/// Returns the platform-specific data directory path for the mutant cache,
/// including a network-specific subdirectory.
/// Creates the directory if it doesn't exist.
async fn get_cache_dir(network: NetworkChoice) -> Result<PathBuf, Error> {
    let data_dir = dirs::data_dir().ok_or_else(|| {
        error!("Could not determine user data directory");
        Error::CacheError("Could not determine user data directory".to_string())
    })?;

    let network_subdir = match network {
        NetworkChoice::Devnet => "devnet",
        NetworkChoice::Mainnet => "mainnet",
    };

    let cache_path = data_dir.join(CACHE_BASE_DIR_NAME).join(network_subdir);

    if !cache_path.exists() {
        debug!("Cache directory does not exist, creating: {:?}", cache_path);
        fs::create_dir_all(&cache_path).await.map_err(|e| {
            error!("Failed to create cache directory {:?}: {}", cache_path, e);
            Error::CacheError(format!("Failed to create cache directory: {}", e))
        })?;
        info!("Created cache directory: {:?}", cache_path);
    }
    Ok(cache_path)
}

/// Returns the full path to the cache file for the specified network.
async fn get_cache_path(network: NetworkChoice) -> Result<PathBuf, Error> {
    let cache_dir = get_cache_dir(network).await?;
    Ok(cache_dir.join(CACHE_FILE_NAME))
}

/// Reads the local index cache for the specified network from the XDG data directory.
/// Returns `Ok(None)` if the cache file does not exist.
pub async fn read_local_index(network: NetworkChoice) -> Result<Option<MasterIndexStorage>, Error> {
    let path = get_cache_path(network).await?;
    debug!(
        "Attempting to read {:?} local index cache from: {:?}",
        network, path
    );

    match File::open(&path).await {
        Ok(_file) => {
            let file_metadata = tokio::fs::metadata(&path).await.map_err(|e| Error::Io(e))?;
            let mut contents: Vec<u8> = Vec::with_capacity(file_metadata.len() as usize);
            let mut file_for_read = File::open(&path).await.map_err(|e| Error::Io(e))?;
            tokio::io::copy(&mut file_for_read, &mut contents)
                .await
                .map_err(|e| Error::Io(e))?;

            match serde_cbor::from_slice(&contents) {
                Ok(index) => {
                    info!(
                        "Successfully read {:?} local index cache from: {:?}",
                        network, path
                    );
                    Ok(Some(index))
                }
                Err(e) => {
                    error!(
                        "Failed to deserialize {:?} cache file {:?}: {}. Cache might be corrupted.",
                        network, path, e
                    );
                    Err(Error::Cbor(e))
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            info!(
                "{:?} local index cache file not found at: {:?}",
                network, path
            );
            Ok(None)
        }
        Err(e) => {
            error!("Failed to open {:?} cache file {:?}: {}", network, path, e);
            Err(Error::Io(e))
        }
    }
}

/// Writes the given index to the local cache file for the specified network
/// in the XDG data directory.
/// Attempts atomic write by writing to a temporary file first.
pub async fn write_local_index(
    index: &MasterIndexStorage,
    network: NetworkChoice,
) -> Result<(), Error> {
    let final_path = get_cache_path(network).await?;
    let cache_dir = final_path.parent().ok_or_else(|| {
        Error::CacheError("Could not determine parent directory for cache file".to_string())
    })?;

    let temp_file_name = format!(".{}.tmp", CACHE_FILE_NAME);
    let temp_path = cache_dir.join(&temp_file_name);

    debug!(
        "Attempting to write {:?} local index cache to temp file: {:?}",
        network, temp_path
    );

    let data_bytes = serde_cbor::to_vec(index).map_err(Error::Cbor)?;

    let write_result = (async {
        let mut temp_file = File::create(&temp_path).await.map_err(|e| {
            error!(
                "Failed to create temporary cache file {:?}: {}",
                temp_path, e
            );
            Error::Io(e)
        })?;
        let mut writer = BufWriter::new(&mut temp_file);
        writer.write_all(&data_bytes).await.map_err(|e| {
            error!(
                "Failed to write data to temporary cache file {:?}: {}",
                temp_path, e
            );
            Error::Io(e)
        })?;
        writer.flush().await.map_err(|e| {
            error!(
                "Failed to flush temporary cache file {:?}: {}",
                temp_path, e
            );
            Error::Io(e)
        })?;
        Ok::<(), Error>(())
    })
    .await;

    if let Err(e) = write_result {
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path).await;
        }
        return Err(e);
    }

    debug!(
        "Renaming temp {:?} cache file {:?} to {:?}",
        network, temp_path, final_path
    );
    fs::rename(&temp_path, &final_path).await.map_err(|e| {
        error!(
            "Failed to rename temporary cache file {:?} to {:?}: {}",
            temp_path, final_path, e
        );
        tokio::spawn(async move {
            if fs::metadata(&temp_path).await.is_ok() {
                let _ = fs::remove_file(&temp_path).await;
            }
        });
        Error::Io(e)
    })?;

    info!(
        "Successfully wrote {:?} local index cache to: {:?}",
        network, final_path
    );
    Ok(())
}

// TODO: Add unit tests for these functions
