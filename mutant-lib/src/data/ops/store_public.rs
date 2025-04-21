use crate::data::chunking::chunk_data;
use crate::data::error::DataError;
use crate::data::manager::DefaultDataManager;
use crate::data::ops::common::{execute_public_upload_tasks, PublicPadType, PublicUploadTaskInput};
use crate::index::error::IndexError;
use crate::index::manager::DefaultIndexManager;
use crate::index::structure::{IndexEntry, PadInfo, PadStatus, PublicUploadInfo};
use crate::internal_events::{invoke_put_callback, PutCallback, PutEvent};
use crate::pad_lifecycle::PadOrigin;
use autonomi::{Bytes, ScratchpadAddress, SecretKey};
use chrono::Utc;
use log::{debug, error, info, warn};
use serde_cbor;
use std::sync::Arc;
use tokio::sync::Mutex;

// Define retry constants locally for public verification
const PUBLIC_CONFIRMATION_RETRY_LIMIT: u32 = 3;
const PUBLIC_CONFIRMATION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Stores data publicly (unencrypted) under a specified name using parallel chunk uploads.
///
/// This operation involves:
/// 1. Checking for name collisions.
/// 2. Chunking the data.
/// 3. Preparing initial `PadInfo` for data and index pads.
/// 4. Inserting an initial `PublicUploadInfo` entry into the index.
/// 5. Executing parallel upload tasks for all pads (data + index).
/// 6. Verifying the index pad upload.
/// 7. Updating the index incrementally as tasks succeed.
pub(crate) async fn store_public_op(
    data_manager: &DefaultDataManager,
    name: String,
    data_bytes: &[u8],
    callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, DataError> {
    info!("DataOps: Starting store_public_op for name '{}'", name);
    let callback_arc = Arc::new(Mutex::new(callback));

    // 1. Check for Name Collision
    {
        let index_copy = data_manager.index_manager.get_index_copy().await?;
        if let Some(entry) = index_copy.index.get(&name) {
            let error_to_return = match entry {
                IndexEntry::PublicUpload(_) => {
                    // Allow overwrite/resume? For now, treat as collision.
                    error!(
                        "Public upload name '{}' collides with an existing public upload.",
                        name
                    );
                    IndexError::PublicUploadNameExists(name.clone())
                }
                IndexEntry::PrivateKey(_) => {
                    error!(
                        "Public upload name '{}' collides with an existing private key.",
                        name
                    );
                    IndexError::KeyExists(name.clone())
                }
            };
            return Err(DataError::Index(error_to_return));
        }
    }

    // 2. Prepare initial index structure and tasks
    let (initial_public_info, tasks_to_run) =
        prepare_public_upload(data_manager.index_manager.clone(), name.clone(), data_bytes).await?;

    let index_pad_address = initial_public_info.index_pad.address;
    let total_chunks = initial_public_info.data_pads.len();

    // 3. Insert initial entry into Index Manager
    // This reserves the name and allows incremental updates.
    data_manager
        .index_manager
        .insert_public_upload_info(name.clone(), initial_public_info)
        .await?;
    debug!("Initial PublicUploadInfo inserted for '{}'", name);

    // 4. Emit Starting event
    if !invoke_put_callback(
        &mut *callback_arc.lock().await,
        PutEvent::Starting {
            total_chunks, // Only count data chunks for progress reporting?
            initial_written_count: 0,
            initial_confirmed_count: 0,
        },
    )
    .await
    .map_err(|e| DataError::CallbackError(e.to_string()))?
    {
        warn!("Public store for '{}' cancelled at start.", name);
        // TODO: Should we remove the initial index entry here?
        return Err(DataError::OperationCancelled);
    }

    // 5. Execute Tasks
    let execution_result = execute_public_upload_tasks(
        data_manager.index_manager.clone(),
        data_manager.network_adapter.clone(),
        name.clone(),
        tasks_to_run,
        callback_arc.clone(),
    )
    .await;

    if let Err(err) = execution_result {
        error!("Public store task execution failed for '{}': {}", name, err);
        // Index entry remains in its partially updated state for potential resume.
        return Err(err);
    }

    // 6. Finalize
    info!(
        "Successfully stored public data '{}' at index {}",
        name,
        index_pad_address // Log the address for user info
    );

    // Final Complete event
    if !invoke_put_callback(&mut *callback_arc.lock().await, PutEvent::Complete)
        .await
        .map_err(|e| DataError::CallbackError(e.to_string()))?
    {
        warn!("Public store for '{}' cancelled after completion.", name);
        // Don't return error here, operation technically succeeded.
    }

    Ok(index_pad_address)
}

/// Prepares the initial `PublicUploadInfo` and the list of tasks required for a public upload.
/// Generates keys, addresses, serializes index content, but does not perform network operations
/// or insert into the index manager.
async fn prepare_public_upload(
    index_manager: Arc<DefaultIndexManager>,
    name: String,
    data_bytes: &[u8],
) -> Result<(PublicUploadInfo, Vec<PublicUploadTaskInput>), DataError> {
    let data_size = data_bytes.len();
    const INITIAL_COUNTER: u64 = 0;

    // --- Chunk Data ---
    let chunk_size = index_manager.get_scratchpad_size().await?;
    if chunk_size == 0 {
        error!("Configured scratchpad size is 0. Cannot proceed with chunking.");
        return Err(DataError::ChunkingError(
            "Invalid scratchpad size (0) configured".to_string(),
        ));
    }
    let chunks = chunk_data(data_bytes, chunk_size)?;
    let total_data_chunks = chunks.len();
    debug!(
        "Public data for '{}' chunked into {} pieces using size {}.",
        name, total_data_chunks, chunk_size
    );

    let mut tasks_to_run = Vec::new();
    let mut data_pad_infos = Vec::with_capacity(total_data_chunks);
    let mut data_pad_addresses = Vec::with_capacity(total_data_chunks);

    // --- Prepare Data Pads ---
    for (i, chunk_data) in chunks.into_iter().enumerate() {
        let sk = SecretKey::random();
        let addr = ScratchpadAddress::new(sk.public_key());
        let sk_bytes = sk.to_bytes().to_vec();
        let chunk_bytes = Bytes::from(chunk_data);

        let pad_info = PadInfo {
            address: addr,
            chunk_index: i,
            status: PadStatus::Generated, // Initial status
            origin: PadOrigin::Generated,
            needs_reverification: false,
            last_known_counter: INITIAL_COUNTER,
            sk_bytes: sk_bytes.clone(), // Store SK bytes temporarily
            receipt: None,
        };

        tasks_to_run.push(PublicUploadTaskInput {
            pad_info: pad_info.clone(), // Clone for the task input
            pad_type: PublicPadType::Data {
                chunk_index: i,
                chunk_data: chunk_bytes,
            },
        });

        data_pad_infos.push(PadInfo {
            sk_bytes: vec![], // Clear SK bytes from the version stored in the index
            ..pad_info        // Use the rest of the prepared info
        });
        data_pad_addresses.push(addr);
    }
    debug!("Prepared {} data pads for '{}'", total_data_chunks, name);

    // --- Prepare Index Pad ---
    let index_sk = SecretKey::random();
    let index_sk_bytes = index_sk.to_bytes().to_vec();
    let index_address = ScratchpadAddress::new(index_sk.public_key());

    let index_data_bytes = serde_cbor::to_vec(&data_pad_addresses)
        .map(Bytes::from)
        .map_err(|e| DataError::Serialization(e.to_string()))?;

    let index_pad_info = PadInfo {
        address: index_address,
        chunk_index: 0, // Index pad doesn't represent a data chunk
        status: PadStatus::Generated,
        origin: PadOrigin::Generated,
        needs_reverification: false,
        last_known_counter: INITIAL_COUNTER,
        sk_bytes: index_sk_bytes.clone(), // Store SK bytes temporarily
        receipt: None,
    };

    tasks_to_run.push(PublicUploadTaskInput {
        pad_info: index_pad_info.clone(), // Clone for the task input
        pad_type: PublicPadType::Index {
            index_data: index_data_bytes,
        },
    });
    debug!("Prepared index pad {} for '{}'", index_address, name);

    // --- Create PublicUploadInfo --- -
    let public_upload_info = PublicUploadInfo {
        index_pad: PadInfo {
            sk_bytes: vec![], // Clear SK bytes from the version stored in the index
            ..index_pad_info  // Use the rest of the prepared info
        },
        size: data_size,
        modified: Utc::now(), // Set initial creation time
        data_pads: data_pad_infos,
    };

    Ok((public_upload_info, tasks_to_run))
}

// Remove placeholder implementation from here, add to adapter
