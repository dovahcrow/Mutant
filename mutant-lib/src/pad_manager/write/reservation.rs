use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::storage_save_mis_from_arc_static;
use crate::storage::Storage;
use crate::utils::retry::retry_operation;
use autonomi::SecretKey;
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
    master_index_storage: Arc<Mutex<MasterIndexStorage>>,
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
    let (mis_addr, _) = storage.get_master_index_info(); // Only need address for saving

    while let Some(result) = stream.next().await {
        match result {
            Ok(pad_info) => {
                // Pad reservation succeeded
                let (pad_address, pad_key_bytes) = pad_info.clone(); // Clone for potential collection
                let log_index = successful_pad_count; // Index for logging before increment

                debug!(
                    "Reservation[{}]: Pad #{} reserved task completed. Adding to free list & saving MIS...",
                    key_for_log,
                    log_index
                );

                // --- Add to free list and save MIS immediately ---
                let pad_secret_key: SecretKey; // Declare outside lock
                {
                    let mut mis_guard = master_index_storage.lock().await;
                    // Convert Vec<u8> back to SecretKey while holding lock (or just before)
                    let key_array: [u8; 32] = match pad_key_bytes.as_slice().try_into() {
                        Ok(arr) => arr,
                        Err(_) => {
                            error!(
                                "Reservation[{}]: Failed to convert key bytes (len {}) to [u8; 32] for pad #{}: Aborting.",
                                key_for_log, pad_key_bytes.len(), log_index
                            );
                            // Release lock before returning error
                            drop(mis_guard);
                            return Err(Error::InternalError(
                                "SecretKey byte conversion failed".to_string(),
                            ));
                        }
                    };
                    // Assign the successfully created SecretKey
                    pad_secret_key = match SecretKey::from_bytes(key_array) {
                        Ok(key) => key,
                        Err(e) => {
                            error!(
                                "Reservation[{}]: Failed to create SecretKey from bytes for pad #{}: {}. Aborting.",
                                key_for_log, log_index, e
                            );
                            // Release lock before returning error
                            drop(mis_guard);
                            return Err(Error::InternalError(format!(
                                "SecretKey creation failed: {}",
                                e
                            )));
                        }
                    };

                    // Add the original pad_info (with Vec<u8> key) to the free list
                    mis_guard.free_pads.push(pad_info.clone());
                    debug!(
                        "Reservation[{}]: Pad {:?} added to in-memory free list (new count: {}). Saving...",
                        key_for_log, pad_address, mis_guard.free_pads.len()
                    );
                    // MIS lock is released here
                }

                // Save Master Index immediately using the reconstructed SecretKey
                let save_result = storage_save_mis_from_arc_static(
                    storage.client(),
                    &mis_addr,             // Use address obtained earlier
                    &pad_secret_key,       // Use the reconstructed SecretKey
                    &master_index_storage, // Pass the Arc<Mutex<>> directly
                )
                .await;

                if let Err(save_err) = save_result {
                    error!(
                        "Reservation[{}]: CRITICAL: Failed to save Master Index after reserving pad #{}: {}. Aborting reservations.",
                        key_for_log, log_index, save_err
                    );
                    // Don't add to collector if save fails
                    // Consider attempting to revert the in-memory free_pads addition? Complex/risky.
                    return Err(save_err); // Return the critical save error
                }

                debug!(
                    "Reservation[{}]: Master Index saved successfully for pad #{}.",
                    key_for_log, log_index
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
                // --- End of successful pad processing ---
            }
            Err(reserve_err) => {
                // Pad reservation failed in the async task
                error!(
                    "Reservation[{}]: Reservation task failed: {}. Aborting remaining reservations.",
                    key_for_log, reserve_err
                );
                // Send the error over the channel to signal failure to the receiver?
                // Or just rely on the function returning Err and the channel closing implicitly?
                // Let's just return Err. The receiver will see the channel close when tx is dropped.
                // if let Err(send_err) = pad_info_tx.send(Err(reserve_err.clone())).await { ... }

                return Err(reserve_err); // Return the reservation error
            }
        }
    }

    // Final check after processing the stream
    if successful_pad_count == num_pads_to_reserve {
        info!(
            "Reservation[{}]: Successfully reserved, individually saved, and sent all {} required pads.",
            key_for_log, num_pads_to_reserve
        );
        // Dropping pad_info_tx here signals successful completion to the receiver
        Ok(())
    } else {
        // Error occurred and was returned above, or loop finished unexpectedly.
        error!(
            "Reservation[{}]: Exited reservation loop unexpectedly. Success count: {}/{}",
            key_for_log, successful_pad_count, num_pads_to_reserve
        );
        // Dropping pad_info_tx here signals abnormal completion
        Err(Error::InternalError(format!(
            "Reservation ended prematurely: {}/{} pads",
            successful_pad_count, num_pads_to_reserve
        )))
    }
    // pad_info_tx is dropped implicitly when the function returns
}
