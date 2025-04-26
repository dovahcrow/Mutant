use crate::network::client::Config;
use crate::network::error::NetworkError;
use crate::network::Network;
use ant_networking::GetRecordError;
use ant_networking::NetworkError as AntNetworkError;
use autonomi::scratchpad::ScratchpadError;
use autonomi::ScratchpadAddress;
use autonomi::SecretKey;
use log::debug;
use log::{error, trace};
use tokio::time::{timeout, Duration};

use super::GetResult;

const GET_TIMEOUT_SECS: u64 = 60 * 60; // 1 hour

/// Retrieves the raw content of a scratchpad from the network.
///
/// This function directly interacts with the Autonomi client to fetch a scratchpad.
/// It handles decryption if an owner_sk is provided.
/// It does not handle retries.
///
/// # Arguments
///
/// * `adapter` - A reference to the `AutonomiNetworkAdapter` containing the initialized client and configuration.
/// * `address` - The address of the scratchpad to retrieve.
/// * `owner_sk` - An optional `SecretKey` for decrypting the data if it's private.
/// # Errors
///
/// Returns `NetworkError` if the client cannot be initialized, fetching fails, or decryption fails.
pub(super) async fn get(
    adapter: &Network,
    address: &ScratchpadAddress,
    owner_sk: Option<&SecretKey>,
) -> Result<GetResult, NetworkError> {
    debug!("Starting get for pad {}", address);
    let client = adapter.get_client(Config::Get).await.map_err(|e| {
        NetworkError::ClientAccessError(format!("Failed to get client from pool: {}", e))
    })?;

    let get_future = client.scratchpad_get(address);

    match timeout(Duration::from_secs(GET_TIMEOUT_SECS), get_future).await {
        Ok(Ok(scratchpad)) => match owner_sk {
            Some(key) => {
                let data = scratchpad.decrypt_data(key).map_err(|e| {
                    NetworkError::InternalError(format!(
                        "Failed to decrypt scratchpad {}: {}",
                        address, e
                    ))
                })?;
                debug!(
                    "Get successful for scratchpad {} with counter {} and data length {}",
                    address,
                    scratchpad.counter(),
                    data.len()
                );
                Ok(GetResult {
                    data: data.to_vec(),
                    counter: scratchpad.counter(),
                    data_encoding: scratchpad.data_encoding(),
                })
            }
            None => {
                debug!(
                    "Get successful for scratchpad {} with counter {} and data length {}",
                    address,
                    scratchpad.counter(),
                    scratchpad.encrypted_data().as_ref().len()
                );
                Ok(GetResult {
                    data: scratchpad.encrypted_data().as_ref().to_vec(),
                    counter: scratchpad.counter(),
                    data_encoding: scratchpad.data_encoding(),
                })
            }
        },
        Ok(Err(e)) => {
            error!("Failed to get scratchpad {}: {}", address, e);
            match e {
                ScratchpadError::Missing => {
                    error!("Scratchpad {} not found", address);
                    Err(NetworkError::GetError(GetRecordError::RecordNotFound))
                }
                ScratchpadError::Network(e) => {
                    error!("Scratchpad error: {}", e);
                    match e {
                        AntNetworkError::GetRecordError(get_error) => {
                            Err(NetworkError::GetError(get_error))
                        }
                        _ => Err(NetworkError::InternalError(format!(
                            "Failed to get scratchpad {}: {}",
                            address, e
                        ))),
                    }
                }
                _ => Err(NetworkError::InternalError(format!(
                    "Failed to get scratchpad {}: {}",
                    address, e
                ))),
            }
        }
        Err(_) => {
            error!("Timeout getting scratchpad {}", address);
            Err(NetworkError::Timeout(format!(
                "Timeout after {} seconds getting scratchpad {}",
                GET_TIMEOUT_SECS, address
            )))
        }
    }
}
