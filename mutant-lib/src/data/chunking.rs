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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_basic() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let chunk_size = 3;
        let chunks = chunk_data(&data, chunk_size).unwrap();
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], vec![1, 2, 3]);
        assert_eq!(chunks[1], vec![4, 5, 6]);
        assert_eq!(chunks[2], vec![7, 8, 9]);
        assert_eq!(chunks[3], vec![10]);
    }

    #[test]
    fn test_chunking_exact_multiple() {
        let data = vec![1, 2, 3, 4, 5, 6];
        let chunk_size = 3;
        let chunks = chunk_data(&data, chunk_size).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], vec![1, 2, 3]);
        assert_eq!(chunks[1], vec![4, 5, 6]);
    }

    #[test]
    fn test_chunking_larger_than_data() {
        let data = vec![1, 2, 3];
        let chunk_size = 10;
        let chunks = chunk_data(&data, chunk_size).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], vec![1, 2, 3]);
    }

    #[test]
    fn test_chunking_empty_data() {
        let data = vec![];
        let chunk_size = 10;
        let chunks = chunk_data(&data, chunk_size).unwrap();
        assert_eq!(chunks.len(), 0); // Should be 0 chunks for empty data
    }

    #[test]
    fn test_chunking_zero_size() {
        let data = vec![1, 2, 3];
        let chunk_size = 0;
        let result = chunk_data(&data, chunk_size);
        assert!(result.is_err());
        match result.err().unwrap() {
            DataError::ChunkingError(msg) => assert_eq!(msg, "Chunk size cannot be zero"),
            _ => panic!("Unexpected error type"),
        }
    }

    #[test]
    fn test_reassemble_basic() {
        let chunks = vec![
            Some(vec![1, 2, 3]),
            Some(vec![4, 5, 6]),
            Some(vec![7, 8, 9]),
            Some(vec![10]),
        ];
        let expected_size = 10;
        let data = reassemble_data(chunks, expected_size).unwrap();
        assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn test_reassemble_empty() {
        let chunks = vec![];
        let expected_size = 0;
        let data = reassemble_data(chunks, expected_size).unwrap();
        assert_eq!(data, Vec::<u8>::new());
    }

    #[test]
    fn test_reassemble_size_mismatch_too_small() {
        let chunks = vec![Some(vec![1, 2]), Some(vec![3])];
        let expected_size = 4; // Expecting more data
        let result = reassemble_data(chunks, expected_size);
        assert!(result.is_err());
        match result.err().unwrap() {
            DataError::ReassemblyError(msg) => {
                assert!(msg.contains("does not match expected size"))
            }
            _ => panic!("Unexpected error type"),
        }
    }

    #[test]
    fn test_reassemble_size_mismatch_too_large() {
        let chunks = vec![Some(vec![1, 2]), Some(vec![3, 4, 5])];
        let expected_size = 3; // Expecting less data
        let result = reassemble_data(chunks, expected_size);
        assert!(result.is_err());
        match result.err().unwrap() {
            DataError::ReassemblyError(msg) => {
                assert!(msg.contains("does not match expected size"))
            }
            _ => panic!("Unexpected error type"),
        }
    }

    #[test]
    fn test_reassemble_missing_chunk() {
        let chunks = vec![Some(vec![1, 2]), None, Some(vec![5, 6])];
        let expected_size = 6;
        let result = reassemble_data(chunks, expected_size);
        assert!(result.is_err());
        match result.err().unwrap() {
            DataError::ReassemblyError(msg) => assert!(msg.contains("Missing chunk at index 1")),
            _ => panic!("Unexpected error type"),
        }
    }
}
