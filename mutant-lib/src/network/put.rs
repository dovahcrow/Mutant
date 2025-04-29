use crate::index::PadInfo;
use crate::network::client::Config;
use crate::network::error::NetworkError;
use crate::network::{Network, PutResult};
use autonomi::client::payment::PaymentOption;
use autonomi::Client;
use autonomi::{Bytes, Scratchpad, ScratchpadAddress, SecretKey, Wallet};
use log::{debug, error, info, trace};
use tokio::time::{timeout, Duration};

const PUT_TIMEOUT_SECS: u64 = 60 * 60; // 1 hour

/// Puts a pre-constructed scratchpad onto the network using `scratchpad_put`.
///
/// This function handles the creation of the Scratchpad object (public or private)
/// and calls the client's `scratchpad_put` method with the appropriate payment option.
/// It does not handle retries.
///
/// # Arguments
///
/// * `client` - A reference to the `AutonomiNetworkAdapter`.
/// * `payment_wallet` - The payment wallet needed for payment.
/// * `pad_info` - Information about the pad, including its key, content type, and counter.
/// * `data` - The raw data bytes to be included in the scratchpad.
/// * `data_encoding` - The encoding type for the data (e.g., content type).
/// * `is_public` - Flag indicating if the scratchpad should be public (no encryption).
///
/// # Errors
///
/// Returns `NetworkError` if:
/// - The client cannot be initialized.
/// - The `SecretKey` cannot be reconstructed from `pad_info`.
/// - The `scratchpad_put` operation fails.
pub(super) async fn put(
    client: &Client,
    payment_wallet: Wallet,
    pad_info: &PadInfo,
    data: &[u8],
    data_encoding: u64,
    is_public: bool,
) -> Result<PutResult, NetworkError> {
    debug!(
        "Starting put for pad {} with data length {}",
        pad_info.address,
        data.len()
    );

    let owner_sk = pad_info.secret_key();

    let data_bytes = Bytes::copy_from_slice(&data);

    let scratchpad = if is_public {
        create_public_scratchpad(
            &owner_sk,
            data_encoding,
            &data_bytes,
            pad_info.last_known_counter,
        )
    } else {
        create_private_scratchpad(
            &owner_sk,
            data_encoding,
            &data_bytes,
            pad_info.last_known_counter,
        )
    };

    let addr = *scratchpad.address();
    trace!("network::put called for address: {}", addr);

    let payment = PaymentOption::Wallet(payment_wallet);

    let put_future = client.scratchpad_put(scratchpad.clone(), payment);

    match timeout(Duration::from_secs(PUT_TIMEOUT_SECS), put_future).await {
        Ok(Ok((cost, received_addr))) => {
            if addr != received_addr {
                error!(
                    "Mismatch between expected addr {} and received addr {} during put",
                    addr, received_addr
                );
            }
            info!("Put successful for scratchpad {} with cost {}", addr, cost);
            Ok(PutResult {
                cost,
                address: addr,
            })
        }
        Ok(Err(e)) => {
            error!("Failed to put scratchpad {}: {}", addr, e);
            Err(NetworkError::InternalError(format!(
                "Failed to put scratchpad {}: {}",
                addr, e
            )))
        }
        Err(_) => {
            error!("Timeout putting scratchpad {}", addr);
            Err(NetworkError::Timeout(format!(
                "Timeout after {} seconds putting scratchpad {}",
                PUT_TIMEOUT_SECS, addr
            )))
        }
    }
}

/// Creates a new public (unencrypted) Scratchpad instance with a valid signature.
fn create_public_scratchpad(
    owner_sk: &SecretKey,
    data_encoding: u64,
    raw_data: &Bytes,
    counter: u64,
) -> Scratchpad {
    trace!("Creating public scratchpad with encoding {}", data_encoding);
    let owner_pk = owner_sk.public_key();
    let address = ScratchpadAddress::new(owner_pk);

    let encrypted_data = raw_data.clone();

    let bytes_to_sign =
        Scratchpad::bytes_for_signature(address, data_encoding, &encrypted_data, counter);
    let signature = owner_sk.sign(&bytes_to_sign);

    Scratchpad::new_with_signature(owner_pk, data_encoding, encrypted_data, counter, signature)
}

/// Creates a new private (encrypted) Scratchpad instance.
fn create_private_scratchpad(
    owner_sk: &SecretKey,
    data_encoding: u64,
    raw_data: &Bytes,
    counter: u64,
) -> Scratchpad {
    Scratchpad::new(owner_sk, data_encoding, raw_data, counter)
}
