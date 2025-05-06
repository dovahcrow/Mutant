use crate::config::NetworkChoice;
use crate::error::Error;
use crate::index::error::IndexError;
use std::fs::File;
use std::io::{BufReader, BufWriter};

use super::{MasterIndex, get_index_file_path};

impl MasterIndex {
    fn new_empty(network_choice: NetworkChoice) -> Self {
        MasterIndex {
            index: Default::default(),
            free_pads: Vec::new(),
            pending_verification_pads: Vec::new(),
            network_choice,
        }
    }

    pub fn new(network_choice: NetworkChoice) -> Self {
        match MasterIndex::load(network_choice) {
            Ok(index) => {
                log::info!("Loaded master index from file for {:?}.", network_choice);
                index
            }
            Err(e) => {
                log::warn!(
                    "Failed to load master index for {:?}, creating a new one. Error: {}",
                    network_choice,
                    e
                );
                MasterIndex::new_empty(network_choice)
            }
        }
    }

    fn load(network_choice: NetworkChoice) -> Result<Self, Error> {
        let path = get_index_file_path(network_choice)?;
        if !path.exists() {
            return Err(Error::Index(IndexError::IndexFileNotFound(
                path.display().to_string(),
            )));
        }
        let file = File::open(&path).map_err(|_e| {
            Error::Index(IndexError::IndexFileNotFound(path.display().to_string()))
        })?;
        let reader = BufReader::new(file);
        let index: MasterIndex = serde_cbor::from_reader(reader)
            .map_err(|e| Error::Index(IndexError::DeserializationError(e.to_string())))?;

        if index.network_choice != network_choice {
            return Err(Error::Index(IndexError::NetworkMismatch {
                x: network_choice,
                y: index.network_choice,
            }));
        }

        Ok(index)
    }

    pub fn save(&self, network_choice: NetworkChoice) -> Result<(), Error> {
        let path = get_index_file_path(network_choice)?;
        let file = File::create(&path).map_err(|_e| {
            Error::Index(IndexError::IndexFileNotFound(path.display().to_string()))
        })?;
        let writer = BufWriter::new(file);
        serde_cbor::to_writer(writer, self)
            .map_err(|e| Error::Index(IndexError::SerializationError(e.to_string())))?;
        log::info!("Saved master index to {}", path.display());
        Ok(())
    }
}
