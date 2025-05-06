use crate::error::Error;
use crate::index::PadStatus;
use crate::internal_events::invoke_put_callback;
use crate::network::Network;
use autonomi::ScratchpadAddress;
use log::{info, warn};
use mutant_protocol::{PutCallback, PutEvent, StorageMode};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::context::Context;
use super::pipeline::write_pipeline;

/// Update a key with new content, preserving the public index pad if applicable.
///
/// For public keys, this function ensures that the public index pad is preserved during updates.
/// This is critical because the public index pad must remain the same to maintain accessibility
/// of the key through its public address. The function:
/// 1. Extracts and preserves the index pad from the existing public key
/// 2. Removes the key (which moves all pads to free_pads or pending_verification_pads)
/// 3. Creates a new key with the updated content
/// 4. Replaces the newly created index pad with the preserved one
/// 5. Updates the index pad data to reflect the new content
pub async fn update(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    key_name: &str,
    content: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    info!("Update for {}", key_name);

    // Special handling for public keys to preserve the index pad
    let mut preserved_index_pad = if public && index.read().await.is_public(key_name) {
        info!("Preserving public index pad for key {}", key_name);
        index.read().await.extract_public_index_pad(key_name)
    } else {
        None
    };

    // Remove the key (this will move all pads to free_pads or pending_verification_pads)
    index.write().await.remove_key(key_name).unwrap();

    // Create a new key with the updated content
    let (pads, chunk_ranges) = index
        .write()
        .await
        .create_key(key_name, &content, mode, public)?;

    info!("Created key {} with {} pads", key_name, pads.len());

    let address = pads[0].address;

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(key_name.to_string()),
        chunk_ranges: Arc::new(chunk_ranges),
        data: content.clone(),
        public,
        preserved_index_pad: None,
        is_index_pad: false, 
    };

    // Write the data pads
    write_pipeline(context, pads.clone(), no_verify, put_callback.clone()).await?;

    // For public keys, we need to write the index pad
    if public {
        let (index_pad, index_data) = index.write().await.populate_index_pad(key_name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        preserved_index_pad = preserved_index_pad.map(|mut old_pad| {
            old_pad.checksum = index_pad.checksum;
            old_pad.size = index_pad.size;
            old_pad.status = PadStatus::Free;
            old_pad.last_known_counter += 1;

            old_pad
        });

        //call update_public_key_with_preserved_index_pad
        if let Some(preserved_index_pad) = &preserved_index_pad {
            index.write().await.update_public_key_with_preserved_index_pad(key_name, preserved_index_pad.clone())?;
        }

        let index_pad_context = Context {
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(key_name.to_string()),
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public,
            preserved_index_pad: preserved_index_pad.clone(),
            is_index_pad: true, // This is an index pad
        };

        // Write the index pad
        write_pipeline(
            index_pad_context,
            vec![preserved_index_pad.unwrap_or(index_pad)],
            no_verify,
            put_callback.clone(),
        )
        .await?;
    }

    // Final completion callback after all pipelines are done
    invoke_put_callback(&put_callback, PutEvent::Complete)
        .await
        .unwrap();

    Ok(address)
}

pub async fn resume(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let pads = index.read().await.get_pads(name);

    if pads.iter().any(|p| p.size > mode.scratchpad_size()) {
        index.write().await.remove_key(name).unwrap();
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback,
        )
        .await;
    }

    let chunk_ranges = index.read().await.chunk_data(&data_bytes, mode.clone());

    if pads.len() != chunk_ranges.len() {
        warn!(
            "Resuming key '{}' with data size mismatch. Index has {} pads, current data requires {}. Forcing rewrite.",
            name,
            pads.len(),
            chunk_ranges.len()
        );
        index.write().await.remove_key(name)?;
        return first_store(
            index,
            network,
            name,
            data_bytes,
            mode,
            public,
            no_verify,
            put_callback.clone(),
        )
        .await;
    }

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        data: data_bytes.clone(),
        chunk_ranges: Arc::new(chunk_ranges),
        public,
        preserved_index_pad: None,
        is_index_pad: false, // This is for data pads, not the index pad
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback.clone()).await?;

    if public {
        let (index_pad, index_data) = index.write().await.populate_index_pad(name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        let index_pad_context = Context {
            // Reuse index and network Arcs
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(name.to_string()), // Reuse name Arc
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public, // Keep public flag
            preserved_index_pad: None, // No preserved index pad for normal operations
            is_index_pad: true, // This is an index pad
        };

        // Call write_pipeline again for the single index pad
        write_pipeline(
            index_pad_context,
            vec![index_pad],
            no_verify,
            put_callback.clone(), // Clone the callback Arc again
        )
        .await?;
    }

    Ok(pads[0].address)
}

pub async fn first_store(
    index: Arc<RwLock<crate::index::master_index::MasterIndex>>,
    network: Arc<Network>,
    name: &str,
    data_bytes: Arc<Vec<u8>>,
    mode: StorageMode,
    public: bool,
    no_verify: bool,
    put_callback: Option<PutCallback>,
) -> Result<ScratchpadAddress, Error> {
    let (pads, chunk_ranges) = index
        .write()
        .await
        .create_key(name, &data_bytes, mode, public)?;

    info!("Created key {} with {} pads", name, pads.len());

    let address = pads[0].address;

    let context = Context {
        index: index.clone(),
        network: network.clone(),
        name: Arc::new(name.to_string()),
        chunk_ranges: Arc::new(chunk_ranges),
        data: data_bytes.clone(),
        public,
        preserved_index_pad: None,
        is_index_pad: false, // This is for data pads, not the index pad
    };

    write_pipeline(context, pads.clone(), no_verify, put_callback.clone()).await?;

    if public {
        let (index_pad, index_data) = index.write().await.populate_index_pad(name)?;
        let index_data_bytes = Arc::new(index_data);
        let index_chunk_ranges = Arc::new(vec![0..index_data_bytes.len()]);

        let index_pad_context = Context {
            // Reuse index and network Arcs
            index: index.clone(),
            network: network.clone(),
            name: Arc::new(name.to_string()), // Reuse name Arc
            chunk_ranges: index_chunk_ranges,
            data: index_data_bytes,
            public, // Keep public flag
            preserved_index_pad: None, // No preserved index pad for normal operations
            is_index_pad: true, // This is an index pad
        };

        // Call write_pipeline again for the single index pad
        write_pipeline(
            index_pad_context,
            vec![index_pad],
            no_verify,
            put_callback.clone(), // Clone the callback Arc again
        )
        .await?;
    }

    // Final completion callback after all pipelines are done
    invoke_put_callback(&put_callback, PutEvent::Complete)
        .await
        .unwrap();

    Ok(address)
}
