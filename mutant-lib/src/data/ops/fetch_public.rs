use crate::data::error::DataError;
use crate::data::{
    manager::DefaultDataManager,
    PUBLIC_DATA_ENCODING,
    PUBLIC_INDEX_ENCODING, // Use constants from parent
};
use crate::internal_events::{invoke_get_callback, GetCallback, GetEvent}; // Import GetEvent variants explicitly

use autonomi::{Bytes, ScratchpadAddress};
use log::{debug, error, info, trace, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Fetches publicly stored data using its index scratchpad address.
///
/// This is the core logic function delegated to by `DataManager::fetch_public`.
pub(crate) async fn fetch_public_op(
    data_manager: &DefaultDataManager,
    public_index_address: ScratchpadAddress,
    callback: Option<GetCallback>,
) -> Result<Bytes, DataError> {
    info!(
        "DataOps: Starting fetch_public_op for index address {}",
        public_index_address
    );
    let callback_arc = Arc::new(Mutex::new(callback));

    // Report index lookup start
    if !invoke_get_callback(&mut *callback_arc.lock().await, GetEvent::IndexLookup)
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
    // Map callback error
    {
        warn!(
            "Public fetch for index {} cancelled during index lookup phase.",
            public_index_address
        );
        return Err(DataError::OperationCancelled);
    }

    // 1. Fetch Public Index Scratchpad
    let index_scratchpad = data_manager
        .network_adapter
        .get_raw_scratchpad(&public_index_address)
        .await
        .map_err(|e| {
            warn!(
                "Failed to fetch public index {}: {}",
                public_index_address, e
            );
            // Use specific error
            DataError::PublicScratchpadNotFound(public_index_address)
        })?;

    // 2. Verify Index Signature and Encoding
    if !index_scratchpad.verify_signature() {
        error!(
            "Invalid signature for public index scratchpad {}",
            public_index_address
        );
        return Err(DataError::InvalidSignature(public_index_address));
    }
    if index_scratchpad.data_encoding() != PUBLIC_INDEX_ENCODING {
        let actual_encoding = index_scratchpad.data_encoding();
        error!(
            "Incorrect data encoding for public index scratchpad {}: expected {}, got {}",
            public_index_address, PUBLIC_INDEX_ENCODING, actual_encoding
        );
        // Use specific error
        return Err(DataError::InvalidPublicIndexEncoding(actual_encoding));
    }
    debug!("Public index scratchpad {} verified.", public_index_address);

    // 3. Deserialize Chunk Addresses
    let chunk_addresses: Vec<ScratchpadAddress> =
        serde_cbor::from_slice(index_scratchpad.encrypted_data())?; // Use `?` with From trait for serde_cbor::Error

    let num_data_chunks = chunk_addresses.len();
    // total_steps is now just the data chunks for the progress bar
    debug!(
        "Deserialized {} data chunk addresses from public index {}. Total steps: {}.",
        num_data_chunks, public_index_address, num_data_chunks
    );

    // Report starting event for GetCallback AFTER index fetch, with only data chunk count
    if !invoke_get_callback(
        &mut *callback_arc.lock().await, // Pass mutable reference to Option<GetCallback>
        GetEvent::Starting {
            total_chunks: num_data_chunks, // Only data chunks
        },
    )
    .await
    .map_err(|e| DataError::CallbackError(e.to_string()))?
    // Map callback error
    {
        warn!(
            "Public fetch for index {} cancelled after index fetch, before data chunks.",
            public_index_address
        );
        return Err(DataError::OperationCancelled);
    }

    // Early exit if only index existed (no data chunks)
    if num_data_chunks == 0 {
        // Report complete event
        if !invoke_get_callback(&mut *callback_arc.lock().await, GetEvent::Complete)
            .await
            .map_err(|e| DataError::CallbackError(e.to_string()))?
        // Map callback error
        {
            warn!(
                "Public fetch for index {} completed (empty data), but cancellation requested after the fact.",
                public_index_address
            );
        }
        return Ok(Bytes::new());
    }

    // 4. Fetch and Concatenate Data Chunks
    let mut assembled_data = Vec::new();
    for (i, chunk_addr) in chunk_addresses.into_iter().enumerate() {
        // Report progress for data chunks (index i: 0 to num_data_chunks-1)
        if !invoke_get_callback(
            &mut *callback_arc.lock().await, // Pass mutable reference
            GetEvent::ChunkFetched {
                chunk_index: i, // Use 0-based index for data chunks
            },
        )
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
        // Map callback error
        {
            warn!(
                "Public fetch for index {} cancelled during data chunk download.",
                public_index_address
            );
            return Err(DataError::OperationCancelled);
        }

        trace!(
            "Fetching public data chunk {}/{} ({})",
            i + 1,
            num_data_chunks, // Log based on data chunks only
            chunk_addr
        );
        let chunk_scratchpad = match data_manager
            .network_adapter
            .get_raw_scratchpad(&chunk_addr)
            .await
        {
            Ok(sp) => sp,
            Err(e) => {
                warn!("Failed to fetch public chunk {}: {}", chunk_addr, e);
                // Map NetworkError to specific DataError
                return Err(DataError::PublicScratchpadNotFound(chunk_addr));
            }
        };

        // Verify chunk signature and encoding
        if !chunk_scratchpad.verify_signature() {
            error!("Invalid signature for public data chunk {}", chunk_addr);
            return Err(DataError::InvalidSignature(chunk_addr));
        }
        if chunk_scratchpad.data_encoding() != PUBLIC_DATA_ENCODING {
            let actual_encoding = chunk_scratchpad.data_encoding();
            error!(
                "Incorrect data encoding for public data chunk {}: expected {}, got {}",
                chunk_addr, PUBLIC_DATA_ENCODING, actual_encoding
            );
            // Use specific error
            return Err(DataError::InvalidPublicDataEncoding(actual_encoding));
        }

        assembled_data.extend_from_slice(chunk_scratchpad.encrypted_data());
        trace!(
            "Appended data from chunk {} ({})",
            i + 1, // Log based on data chunks only
            chunk_addr
        );
    }

    let final_size = assembled_data.len();

    info!(
        "Successfully fetched index and {} public data chunks ({} bytes) for index {}",
        num_data_chunks, final_size, public_index_address
    );

    // Report complete event
    if !invoke_get_callback(&mut *callback_arc.lock().await, GetEvent::Complete)
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
    // Map callback error
    {
        // Log if cancelled after completion, but don't error out, data is fetched.
        warn!(
            "Public fetch for index {} completed, but cancellation requested after the fact.",
            public_index_address
        );
    }

    Ok(Bytes::from(assembled_data))
}
