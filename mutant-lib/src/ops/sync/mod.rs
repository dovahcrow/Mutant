use crate::ops::utils::{derive_master_index_info, DATA_ENCODING_MASTER_INDEX};
use crate::error::Error;
use crate::events::{SyncCallback, SyncEvent};
use crate::index::master_index::MasterIndex;
use crate::index::{PadInfo, PadStatus};
use crate::internal_events::invoke_sync_callback;
use crate::network::client::Config;
use crate::network::{Network, NetworkError};
use ant_networking::GetRecordError;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use mutant_protocol::SyncResult;

pub(super) async fn sync(
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    force: bool,
    sync_callback: Option<SyncCallback>,
) -> Result<SyncResult, Error> {
    let mut sync_result = SyncResult {
        nb_keys_added: 0,
        nb_keys_updated: 0,
        nb_free_pads_added: 0,
        nb_pending_pads_added: 0,
    };
    let callback = sync_callback.clone();

    invoke_sync_callback(&callback, SyncEvent::FetchingRemoteIndex)
        .await
        .unwrap();

    let owner_secret_key_data = network.secret_key();
    let (owner_address, owner_secret_key) =
        derive_master_index_info(&owner_secret_key_data.to_hex())?;

    let client_guard_get = network
        .get_client(Config::Get)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client_get = &*client_guard_get;

    let (remote_index, remote_index_counter) = match network
        .get(client_get, &owner_address, Some(&owner_secret_key))
        .await
    {
        Ok(get_result) => {
            let remote_index = if force {
                MasterIndex::new(network.network_choice())
            } else {
                serde_cbor::from_slice(&get_result.data).unwrap()
            };

            (remote_index, get_result.counter)
        }
        Err(_e) => (MasterIndex::new(network.network_choice()), 0),
    };
    drop(client_guard_get);

    invoke_sync_callback(&callback, SyncEvent::Merging)
        .await
        .unwrap();

    let mut local_index = index.write().await;

    if !force {
        for (key, remote_entry) in remote_index.list() {
            let local_entry = local_index.get_entry(&key);
            if local_entry.is_none() {
                local_index.add_entry(&key, remote_entry.clone())?; // Clone remote_entry
                sync_result.nb_keys_added += 1;
            } else {
                if local_index.update_entry(&key, remote_entry.clone())? {
                    // Clone remote_entry
                    sync_result.nb_keys_updated += 1;
                }
            }
        }

        let mut free_pads_to_add = Vec::new();
        let mut pending_pads_to_add = Vec::new();

        for pad in remote_index.export_raw_pads_private_key()? {
            if local_index.pad_exists(&pad.address) {
                continue;
            }
            if pad.status == PadStatus::Generated {
                pending_pads_to_add.push(pad);
                sync_result.nb_pending_pads_added += 1;
            } else {
                free_pads_to_add.push(pad);
                sync_result.nb_free_pads_added += 1;
            }
        }

        local_index.import_raw_pads_private_key(free_pads_to_add)?;
        local_index.import_raw_pads_private_key(pending_pads_to_add)?;
    }

    let serialized_index = serde_cbor::to_vec(&*local_index).unwrap(); // Deref local_index
    drop(local_index); // Drop the write lock before potential network calls

    let client_guard_put = network
        .get_client(Config::Put)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client_put = &*client_guard_put;

    invoke_sync_callback(&callback, SyncEvent::PushingRemoteIndex)
        .await
        .unwrap();

    let pad_info = PadInfo {
        address: owner_address,
        status: PadStatus::Confirmed,
        chunk_index: 0,
        size: serialized_index.len(),
        last_known_counter: remote_index_counter + 1,
        sk_bytes: owner_secret_key.to_bytes().to_vec(),
        checksum: 0,
    };

    network
        .put(
            client_put,
            &pad_info,
            &serialized_index,
            DATA_ENCODING_MASTER_INDEX,
            false,
        )
        .await?;
    drop(client_guard_put);

    let client_guard_verify = network
        .get_client(Config::Get)
        .await
        .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?;
    let client_verify = &*client_guard_verify;

    invoke_sync_callback(&callback, SyncEvent::VerifyingRemoteIndex)
        .await
        .unwrap();

    let mut retries = 10;

    loop {
        match network
            .get(client_verify, &owner_address, Some(&owner_secret_key))
            .await
        {
            Ok(get_result) => {
                if get_result.data != serialized_index {
                } else if get_result.counter != remote_index_counter + 1 {
                } else {
                    break Ok(());
                }
            }
            Err(_e) => {}
        };

        if retries == 0 {
            break Err(Error::Network(
                NetworkError::GetError(GetRecordError::RecordNotFound).into(),
            ));
        }

        retries -= 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }?;

    drop(client_guard_verify);

    // Reacquire lock to update the index in memory (optional, depending on desired consistency)
    // *index.write().await = local_index_data; // If we had cloned the data before dropping lock

    invoke_sync_callback(&callback, SyncEvent::Complete)
        .await
        .unwrap();

    Ok(sync_result)
}
