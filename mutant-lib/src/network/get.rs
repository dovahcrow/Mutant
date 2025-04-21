use crate::network::error::NetworkError;
use crate::network::Network;
use autonomi::ScratchpadAddress;
use autonomi::SecretKey;
use log::{error, trace};

use super::GetResult;

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
    trace!("network::get called for address: {}", address);
    let client = adapter.get_or_init_client().await?;

    match client.scratchpad_get(address).await {
        Ok(scratchpad) => match owner_sk {
            Some(key) => {
                let data = scratchpad.decrypt_data(key).map_err(|e| {
                    NetworkError::InternalError(format!(
                        "Failed to decrypt scratchpad {}: {}",
                        address, e
                    ))
                })?;
                Ok(GetResult {
                    data: data.to_vec(),
                    counter: scratchpad.counter(),
                    data_encoding: scratchpad.data_encoding(),
                })
            }
            None => Ok(GetResult {
                data: scratchpad.encrypted_data().as_ref().to_vec(),
                counter: scratchpad.counter(),
                data_encoding: scratchpad.data_encoding(),
            }),
        },
        Err(e) => {
            error!("Failed to get scratchpad {}: {}", address, e);
            Err(NetworkError::InternalError(format!(
                "Failed to get scratchpad {}: {}",
                address, e
            )))
        }
    }
}
