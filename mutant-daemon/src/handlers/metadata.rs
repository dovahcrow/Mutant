use std::sync::Arc;

use crate::error::Error as DaemonError;
use mutant_lib::storage::{IndexEntry, PadStatus};
use mutant_lib::MutAnt;
use mutant_protocol::{
    ErrorResponse, KeyDetails, ListKeysRequest, ListKeysResponse, Response, StatsRequest,
    StatsResponse,
};

use super::common::UpdateSender;

pub(crate) async fn handle_list_keys(
    _req: ListKeysRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling ListKeys request");

    // Use mutant.list() to get detailed IndexEntry data
    let index_result = mutant.list().await;

    let response = match index_result {
        Ok(index_map) => {
            tracing::info!("Found {} keys", index_map.len());
            let details: Vec<KeyDetails> = index_map
                .into_iter()
                .map(|(key, entry)| match entry {
                    IndexEntry::PrivateKey(pads) => {
                        let total_size = pads.iter().map(|p| p.size).sum::<usize>();
                        let pad_count = pads.len();
                        let confirmed_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count();
                        KeyDetails {
                            key,
                            total_size,
                            pad_count,
                            confirmed_pads,
                            is_public: false,
                            public_address: None,
                        }
                    }
                    IndexEntry::PublicUpload(index_pad, pads) => {
                        let data_size = pads.iter().map(|p| p.size).sum::<usize>();
                        let total_size = data_size + index_pad.size;
                        let pad_count = pads.len() + 1; // +1 for index pad
                        let confirmed_data_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count();
                        let index_pad_confirmed = if index_pad.status == PadStatus::Confirmed {
                            1
                        } else {
                            0
                        };
                        let confirmed_pads = confirmed_data_pads + index_pad_confirmed;
                        KeyDetails {
                            key,
                            total_size,
                            pad_count,
                            confirmed_pads,
                            is_public: true,
                            public_address: Some(index_pad.address.to_hex()),
                        }
                    }
                })
                .collect();

            Response::ListKeys(ListKeysResponse { keys: details })
        }
        Err(e) => {
            tracing::error!("Failed to list keys from mutant-lib: {}", e);
            Response::Error(ErrorResponse {
                error: format!("Failed to retrieve key list: {}", e),
                original_request: None, // Cannot easily get original string here yet
            })
        }
    };

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}

pub(crate) async fn handle_stats(
    _req: StatsRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    tracing::debug!("Handling Stats request");

    // Call the method which returns StorageStats directly
    let stats = mutant.get_storage_stats().await;
    tracing::info!("Retrieved storage stats successfully: {:?}", stats);

    // Adapt the fields from mutant_lib::StorageStats to mutant_protocol::StatsResponse
    let response = Response::Stats(StatsResponse {
        total_keys: stats.nb_keys,
        // total_size: stats.total_size, // This field does not exist in StorageStats
        total_pads: stats.total_pads,
        occupied_pads: stats.occupied_pads,
        free_pads: stats.free_pads,
        pending_verify_pads: stats.pending_verification_pads,
    });

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}
