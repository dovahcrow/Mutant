use crate::data::error::DataError;
use log::trace;

/// Chunks the input data into smaller pieces based on the specified chunk size.
/// The last chunk may be smaller than the chunk_size.
pub(crate) fn chunk_data(data: &[u8], chunk_size: usize) -> Result<Vec<Vec<u8>>, DataError> {
    trace!(
        "Chunking data of size {} into chunks of size {}",
        data.len(),
        chunk_size
    );
    if chunk_size == 0 {
        return Err(DataError::ChunkingError(
            "Chunk size cannot be zero".to_string(),
        ));
    }

    // Use iterator-based chunking for efficiency
    let chunks: Vec<Vec<u8>> = data
        .chunks(chunk_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    trace!("Data chunked into {} pieces", chunks.len());
    Ok(chunks)
}

/// Reassembles data chunks into the original byte vector.
/// Validates that the reassembled size matches the expected size.
pub(crate) fn reassemble_data(
    mut chunks: Vec<Option<Vec<u8>>>, // Use Option to handle potential missing chunks during fetch
    expected_size: usize,
) -> Result<Vec<u8>, DataError> {
    trace!(
        "Reassembling {} chunks, expected final size: {}",
        chunks.len(),
        expected_size
    );

    // Sort chunks based on their assumed index (position in the Vec)
    // This assumes the caller provides chunks in the correct order or handles ordering.
    // If fetch returns chunks out of order, sorting based on PadInfo.chunk_index is needed *before* calling this.
    // For simplicity here, assume they are ordered correctly but might have gaps (None).

    let mut reassembled_data = Vec::with_capacity(expected_size); // Pre-allocate close to expected size

    for (i, chunk_opt) in chunks.iter_mut().enumerate() {
        match chunk_opt.take() {
            // Use take() to consume the Option
            Some(chunk) => {
                reassembled_data.extend_from_slice(&chunk);
            }
            None => {
                // A chunk is missing!
                return Err(DataError::ReassemblyError(format!(
                    "Missing chunk at index {} during reassembly",
                    i
                )));
            }
        }
    }

    // Final size check
    if reassembled_data.len() != expected_size {
        return Err(DataError::ReassemblyError(format!(
            "Reassembled data size ({}) does not match expected size ({})",
            reassembled_data.len(),
            expected_size
        )));
    }

    trace!(
        "Data reassembled successfully to size {}",
        reassembled_data.len()
    );
    Ok(reassembled_data)
}
