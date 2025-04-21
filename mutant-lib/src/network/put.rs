use std::time::Duration;

use crate::index::structure::PadInfo;
use crate::network::error::NetworkError;
use crate::network::wallet::payment_option_for_pad;
use crate::network::{AutonomiNetworkAdapter, PutResult};
use autonomi::client::payment::PaymentOption;
use autonomi::{AttoTokens, Bytes, Scratchpad, ScratchpadAddress, SecretKey};
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
    is_public: bool, // Caller needs to determine this
) -> Result<PutResult, NetworkError> {
    let client = adapter.get_or_init_client().await?;

    // Reconstruct SecretKey from bytes stored in PadInfo
    let owner_sk = pad_info.secret_key();

    // i want the data to be a Scratchpad::MAX_SIZE
    let data_bytes = Bytes::copy_from_slice(&data);

    let scratchpad = if is_public {
        create_public_scratchpad(
            &owner_sk,
            data_encoding,
            &data_bytes,
            pad_info.last_known_counter, // Assuming counter is needed for public
        )
    } else {
        create_private_scratchpad(
            &owner_sk,
            data_encoding,
            &data_bytes,
            pad_info.last_known_counter, // Use the counter from PadInfo
        )
    };

    let addr = *scratchpad.address();
    trace!("network::put called for address: {}", addr);

    let payment = payment_option_for_pad(pad_info, &adapter.wallet)?;
    match &payment {
        PaymentOption::Wallet(_) => println!("payment: Wallet"),
        PaymentOption::Receipt(receipt) => println!("payment: Receipt {:#?}", receipt),
    }

    let (cost, addr) = client
        .scratchpad_put(scratchpad.clone(), payment)
        .await
        .map_err(|e| {
            error!("Failed to put scratchpad {}: {}", addr, e);
            NetworkError::InternalError(format!("Failed to put scratchpad {}: {}", addr, e))
        })?;

    println!("cost: {:#?}", cost);

    tokio::time::sleep(Duration::from_secs(10)).await;

    let mut pad_info = pad_info.clone();
    pad_info.address = addr;
    pad_info.size = scratchpad.size();

    // if pad_info.receipt is None, fetch the receipt from the network.  use fetch_receipt from the adapter
    let receipt = if pad_info.receipt.is_none() {
        adapter.fetch_receipt(&pad_info).await?
    } else {
        pad_info.receipt.clone().unwrap()
    };

    Ok(PutResult {
        cost,
        address: addr,
        counter: scratchpad.counter(),
        data_encoding,
        receipt,
        size_tmp: scratchpad.size(),
    })
}

/// Creates a new public (unencrypted) Scratchpad instance with a valid signature.
/// Moved from the old adapter.rs.
fn create_public_scratchpad(
    owner_sk: &SecretKey,
    data_encoding: u64,
    raw_data: &Bytes,
    counter: u64,
) -> Scratchpad {
    trace!("Creating public scratchpad with encoding {}", data_encoding);
    let owner_pk = owner_sk.public_key();
    let address = ScratchpadAddress::new(owner_pk);

    // Data is passed directly as "encrypted_data" but is not actually encrypted.
    let encrypted_data = raw_data.clone();

    let bytes_to_sign =
        Scratchpad::bytes_for_signature(address, data_encoding, &encrypted_data, counter);
    let signature = owner_sk.sign(&bytes_to_sign);

    Scratchpad::new_with_signature(owner_pk, data_encoding, encrypted_data, counter, signature)
}

fn create_private_scratchpad(
    owner_sk: &SecretKey,
    data_encoding: u64,
    raw_data: &Bytes,
    counter: u64,
) -> Scratchpad {
    Scratchpad::new(owner_sk, data_encoding, raw_data, counter)
}
