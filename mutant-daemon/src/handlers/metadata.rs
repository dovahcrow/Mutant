use std::sync::Arc;

use crate::error::Error as DaemonError;
use mutant_lib::storage::{IndexEntry, PadStatus};
use mutant_lib::MutAnt;
use mutant_protocol::{
    ErrorResponse, KeyDetails, ListKeysRequest, ListKeysResponse, Response, StatsRequest,
    StatsResponse, WalletBalanceRequest, WalletBalanceResponse,
};

use super::common::UpdateSender;

pub(crate) async fn handle_list_keys(
    _req: ListKeysRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    log::debug!("Handling ListKeys request");

    // Use mutant.list() to get detailed IndexEntry data
    let index_result = mutant.list().await;

    let response = match index_result {
        Ok(index_map) => {
            log::info!("Found {} keys", index_map.len());
            let details: Vec<KeyDetails> = index_map
                .into_iter()
                .map(|(key, entry)| match entry {
                    IndexEntry::PrivateKey(pads) => {
                        let total_size = pads.iter().map(|p| p.size).sum::<usize>() as u64;
                        let pad_count = pads.len() as u64;
                        let confirmed_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count() as u64;
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
                        let total_size = (data_size + index_pad.size) as u64;
                        let pad_count = (pads.len() + 1) as u64; // +1 for index pad
                        let confirmed_data_pads = pads
                            .iter()
                            .filter(|p| p.status == PadStatus::Confirmed)
                            .count();
                        let index_pad_confirmed = if index_pad.status == PadStatus::Confirmed {
                            1
                        } else {
                            0
                        };
                        let confirmed_pads = (confirmed_data_pads + index_pad_confirmed) as u64;
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
            log::error!("Failed to list keys from mutant-lib: {}", e);
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
    log::debug!("Handling Stats request");

    // Call the method which returns StorageStats directly
    let stats = mutant.get_storage_stats().await;
    log::info!("Retrieved storage stats successfully: {:?}", stats);

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

pub(crate) async fn handle_wallet_balance(
    _req: WalletBalanceRequest,
    update_tx: UpdateSender,
    mutant: Arc<MutAnt>,
) -> Result<(), DaemonError> {
    log::debug!("Handling WalletBalance request");

    // Get the wallet from MutAnt
    let wallet_result = mutant.get_wallet().await;

    let response = match wallet_result {
        Ok(wallet) => {
            // Get token and gas balances
            let token_balance_result = wallet.balance_of_tokens().await;
            let gas_balance_result = wallet.balance_of_gas_tokens().await;

            match (token_balance_result, gas_balance_result) {
                (Ok(token_balance), Ok(gas_balance)) => {
                    log::info!("Retrieved wallet balances - Tokens: {}, Gas: {}", token_balance, gas_balance);
                    Response::WalletBalance(WalletBalanceResponse {
                        token_balance: token_balance.to_string(),
                        gas_balance: gas_balance.to_string(),
                    })
                }
                (Err(token_err), Ok(_)) => {
                    log::error!("Failed to get token balance: {}", token_err);
                    Response::Error(ErrorResponse {
                        error: format!("Failed to get token balance: {}", token_err),
                        original_request: None,
                    })
                }
                (Ok(_), Err(gas_err)) => {
                    log::error!("Failed to get gas balance: {}", gas_err);
                    Response::Error(ErrorResponse {
                        error: format!("Failed to get gas balance: {}", gas_err),
                        original_request: None,
                    })
                }
                (Err(token_err), Err(gas_err)) => {
                    log::error!("Failed to get both balances - Token: {}, Gas: {}", token_err, gas_err);
                    Response::Error(ErrorResponse {
                        error: format!("Failed to get wallet balances - Token: {}, Gas: {}", token_err, gas_err),
                        original_request: None,
                    })
                }
            }
        }
        Err(e) => {
            log::error!("Failed to get wallet: {}", e);
            Response::Error(ErrorResponse {
                error: format!("Failed to get wallet: {}", e),
                original_request: None,
            })
        }
    };

    update_tx
        .send(response)
        .map_err(|e| DaemonError::Internal(format!("Update channel send error: {}", e)))?;

    Ok(())
}
