use crate::network::error::NetworkError;
use crate::network::AutonomiNetworkAdapter;
use autonomi::ScratchpadAddress;
use log::{error, trace};

use super::GetResult;

/// Retrieves the raw content of a scratchpad from the network.
///
/// This function directly interacts with the Autonomi client to fetch a scratchpad.
/// It does not handle retries.
///
/// # Arguments
///
/// * `adapter` - A reference to the `AutonomiNetworkAdapter` containing the initialized client and configuration.
/// * `address` - The address of the scratchpad to retrieve.
/// * `is_public` - Whether the scratchpad is public.
/// # Errors
///
/// Returns `NetworkError` if the client cannot be initialized or if fetching the scratchpad fails.
pub(super) async fn get(
    adapter: &AutonomiNetworkAdapter,
    address: &ScratchpadAddress,
    is_public: bool,
) -> Result<GetResult, NetworkError> {
    trace!("network::get called for address: {}", address);
    let client = adapter.get_or_init_client().await?;

    match client.scratchpad_get(address).await {
        Ok(scratchpad) => {
            if is_public {
                Ok(GetResult {
                    data: scratchpad.encrypted_data().as_ref().to_vec(),
                    counter: scratchpad.counter(),
                    data_encoding: scratchpad.data_encoding(),
                })
            } else {
                let data = scratchpad.decrypt_data(&adapter.secret_key).map_err(|e| {
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
        }
        Err(e) => {
            error!("Failed to get scratchpad {}: {}", address, e);
            Err(NetworkError::InternalError(format!(
                "Failed to get scratchpad {}: {}",
                address, e
            )))
        }
    }
}
