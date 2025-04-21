use crate::index::structure::PadInfo;
use crate::network::error::NetworkError;
use crate::network::{AutonomiNetworkAdapter, PutResult};
use autonomi::client::payment::PaymentOption;
use autonomi::{Bytes, Scratchpad, ScratchpadAddress, SecretKey};
use log::{error, trace};

/// Puts a pre-constructed scratchpad onto the network using `scratchpad_put`.
///
/// This function handles the creation of the Scratchpad object (public or private)
/// and calls the client's `scratchpad_put` method with the appropriate payment option.
/// It does not handle retries.
///
/// # Arguments
///
/// * `adapter` - A reference to the `AutonomiNetworkAdapter`.
/// * `pad_info` - Information about the pad, including its key, content type, and counter.
/// * `data` - The raw data bytes to be included in the scratchpad.
/// * `payment` - The payment option (`Wallet` for new pads, `Receipt` for updates).
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
    adapter: &AutonomiNetworkAdapter,
    pad_info: &PadInfo,
    data: &[u8],
    data_encoding: u64,
    is_public: bool,
) -> Result<PutResult, NetworkError> {
    let client = adapter.get_or_init_client().await?;

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

    let payment = PaymentOption::Wallet(adapter.wallet.clone());

    let (cost, addr) = client
        .scratchpad_put(scratchpad.clone(), payment)
        .await
        .map_err(|e| {
            error!("Failed to put scratchpad {}: {}", addr, e);
            NetworkError::InternalError(format!("Failed to put scratchpad {}: {}", addr, e))
        })?;

    Ok(PutResult {
        cost,
        address: addr,
    })
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
