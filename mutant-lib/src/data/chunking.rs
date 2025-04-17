use crate::data::error::DataError;
use log::trace;

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

    let chunks: Vec<Vec<u8>> = data
        .chunks(chunk_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    trace!("Data chunked into {} pieces", chunks.len());
    Ok(chunks)
}

pub(crate) fn reassemble_data(
    mut chunks: Vec<Option<Vec<u8>>>,
    expected_size: usize,
) -> Result<Vec<u8>, DataError> {
    trace!(
        "Reassembling {} chunks, expected final size: {}",
        chunks.len(),
        expected_size
    );

    let mut reassembled_data = Vec::with_capacity(expected_size);

    for (i, chunk_opt) in chunks.iter_mut().enumerate() {
        match chunk_opt.take() {
            Some(chunk) => {
                reassembled_data.extend_from_slice(&chunk);
            }
            None => {
                return Err(DataError::ReassemblyError(format!(
                    "Missing chunk at index {} during reassembly",
                    i
                )));
            }
        }
    }

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
