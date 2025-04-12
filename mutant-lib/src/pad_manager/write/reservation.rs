use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::Storage;
use crate::utils::retry::retry_operation;
use futures::stream::{self, StreamExt};
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

pub(super) const MAX_CONCURRENT_RESERVATIONS: usize = 10;

/// Reserves a specified number of new pads concurrently.
/// Saves the Master Index after each successful reservation and sends PadInfo via channel.
pub(super) async fn reserve_new_pads_and_collect(
    key_for_log: &str,
    num_pads_to_reserve: usize,
    shared_callback: Arc<Mutex<Option<PutCallback>>>,
    pad_info_tx: mpsc::Sender<Result<PadInfo, Error>>,
    storage: Arc<Storage>,
    _master_index_storage: Arc<Mutex<MasterIndexStorage>>,
) -> Result<(), Error> {
    info!(
        "Reservation[{}]: Reserving {} new pads (max concurrent: {})... Saving after each success...",
        key_for_log, num_pads_to_reserve, MAX_CONCURRENT_RESERVATIONS
    );

    // Initial callback event for the whole batch
    {
        let mut cb_guard = shared_callback.lock().await;
        invoke_callback(
            &mut *cb_guard,
            PutEvent::ReservingPads {
                count: num_pads_to_reserve as u64,
            },
        )
        .await?;
    }

    // Create a stream of reservation tasks
    let results_stream = stream::iter(0..num_pads_to_reserve).map(|i| {
        // Clone only what's needed for the reservation task itself
        let storage_clone = storage.clone();
        let cb_clone = shared_callback.clone(); // Clone for the async block

        async move {
            // This block only performs reservation and reports PadReserved
            let pad_index = i;

            let reserve_result = retry_operation(
                &format!("Reserve Pad #{}", pad_index),
                || async {
                    let (pad_address, pad_key) =
                        storage_clone.create_scratchpad_internal_raw(&[], 0).await?;
                    // Return key as bytes
                    Ok((pad_address, pad_key.to_bytes().to_vec()))
                },
                |_e: &Error| true, // Retry on any storage error
            )
            .await;

            // Report PadReserved result via callback
            {
                let mut cb_guard = cb_clone.lock().await;
                invoke_callback(
                    &mut *cb_guard,
                    PutEvent::PadReserved {
                        index: pad_index as u64,
                        total: num_pads_to_reserve as u64,
                        result: reserve_result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|e| e.to_string()), // Convert error to string for event
                    },
                )
                .await
                .ok(); // Ignore callback error here, main error handled below
            }

            // Return the result of the reservation attempt (Ok(PadInfo) or Err(Error))
            reserve_result
        }
    });

    // Process the results sequentially: save MIS on success, abort on failure
    let mut stream = Box::pin(results_stream.buffer_unordered(MAX_CONCURRENT_RESERVATIONS));
    let mut successful_pad_count = 0;
    let (_mis_addr, _) = storage.get_master_index_info(); // Only need address for saving

    while let Some(result) = stream.next().await {
        match result {
            Ok(pad_info) => {
                // Pad reservation succeeded
                let (pad_address, _pad_key_bytes) = pad_info.clone(); // Clone for logging/sending
                let log_index = successful_pad_count; // Index for logging before increment

                debug!(
                    "Reservation[{}]: Pad #{} ({:?}) reserved task completed. Sending to writer.",
                    key_for_log, log_index, pad_address
                );

                // Send successful PadInfo over the channel
                if let Err(send_err) = pad_info_tx.send(Ok(pad_info.clone())).await {
                    error!(
                        "Reservation[{}]: Failed to send successful PadInfo for pad #{} over channel: {}. Aborting.",
                        key_for_log, log_index, send_err
                    );
                    // Treat channel send failure as critical, as the uploader won't get the pad info
                    return Err(Error::InternalError(format!(
                        "Channel send error: {}",
                        send_err
                    )));
                }

                successful_pad_count += 1;
            }
            Err(reserve_err) => {
                // Pad reservation failed
                error!(
                    "Reservation[{}]: Reservation task failed: {}. Aborting remaining reservations.",
                    key_for_log, reserve_err
                );
                return Err(reserve_err); // Return the reservation error
            }
        }
    }

    // Final check after processing the stream
    if successful_pad_count == num_pads_to_reserve {
        info!(
            "Reservation[{}]: Successfully reserved and sent all {} required pads to writer.",
            key_for_log, num_pads_to_reserve
        );
        Ok(())
    } else {
        error!(
            "Reservation[{}]: Exited reservation loop unexpectedly. Success count: {}/{}",
            key_for_log, successful_pad_count, num_pads_to_reserve
        );
        Err(Error::InternalError(format!(
            "Reservation ended prematurely: {}/{} pads",
            successful_pad_count, num_pads_to_reserve
        )))
    }
    // pad_info_tx is dropped implicitly when the function returns, closing the channel.
}
