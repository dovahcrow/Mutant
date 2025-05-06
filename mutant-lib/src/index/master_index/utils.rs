use crate::config::NetworkChoice;
use crate::error::Error;
use crate::index::error::IndexError;
use std::fs;
use std::path::PathBuf;
use xdg::BaseDirectories;

// Helper function to get the XDG data directory for Mutant
pub fn get_mutant_data_dir() -> Result<PathBuf, Error> {
    let xdg_dirs = BaseDirectories::with_prefix("mutant")
        .map_err(|e| Error::Index(IndexError::IndexFileNotFound(e.to_string())))?;
    let data_dir = xdg_dirs.get_data_home();
    fs::create_dir_all(&data_dir)
        .map_err(|e| Error::Index(IndexError::IndexFileNotFound(e.to_string())))?; // Ensure the directory exists
    Ok(data_dir)
}

// Helper function to get the full path for the index file
pub fn get_index_file_path(network_choice: NetworkChoice) -> Result<PathBuf, Error> {
    let data_dir = get_mutant_data_dir()?;
    let filename = match network_choice {
        NetworkChoice::Mainnet => "master_index_mainnet.cbor",
        NetworkChoice::Devnet => "master_index_devnet.cbor",
        NetworkChoice::Alphanet => "master_index_alphanet.cbor",
    };
    Ok(data_dir.join(filename))
}
