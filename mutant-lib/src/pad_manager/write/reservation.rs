use super::PadInfoAlias;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::mutant::data_structures::MasterIndexStorage;
use crate::storage::Storage as BaseStorage;
use crate::utils::retry::retry_operation;
use futures::stream::StreamExt;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;
use tokio_stream as stream; // Only import the alias

pub(super) const MAX_CONCURRENT_RESERVATIONS: usize = 5;

/// Reserves a specified number of new pads concurrently.
/// Saves the Master Index after each successful reservation and sends PadInfo via channel.
pub(super) async fn reserve_new_pads_and_collect(
    key_for_log: &str,
    num_pads_to_reserve: usize,
    shared_callback: Arc<Mutex<Option<PutCallback>>>,
    pad_info_tx: Sender<Result<PadInfoAlias, Error>>,
    storage: Arc<BaseStorage>,
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

            // Return the result of the reservation attempt (Ok(PadInfoAlias) or Err(Error))
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
                // Reservation successful
                successful_pad_count += 1;
                debug!(
                    "Reservation[{}]: Successfully reserved pad #{} ({}/{})",
                    key_for_log,
                    successful_pad_count, // Use count instead of index for log message
                    successful_pad_count,
                    num_pads_to_reserve
                );

                // --- Add to free_pads in MIS and Save MIS --- //
                {
                    let mut mis_guard = master_index_storage.lock().await;
                    mis_guard.free_pads.push(pad_info.clone()); // Clone needed as pad_info is sent
                    let mis_data_to_write = mis_guard.clone();
                    drop(mis_guard); // Release lock before potentially slow I/O

                    let network_choice = storage.get_network_choice();
                    if let Err(e) =
                        crate::cache::write_local_index(&mis_data_to_write, network_choice).await
                    {
                        error!("Reservation[{}]: Failed to persist MIS after reserving pad #{}: {}. Continuing...", key_for_log, successful_pad_count, e);
                        // Don't abort the whole reservation process, just log the persistence error.
                    }
                } // End scope for MIS lock

                // Send the successfully reserved PadInfoAlias to the write task
                if pad_info_tx.send(Ok(pad_info)).await.is_err() {
                    error!(
                        "Reservation[{}]: Failed to send reserved pad info to writer task. Channel closed?",
                        key_for_log
                    );
                    // Abort if we can't communicate with the writer
                    return Err(Error::InternalError(
                        "Pad reservation channel closed unexpectedly".to_string(),
                    ));
                }

                // Optional: Update ReservationProgress callback
                {
                    let mut cb_guard = shared_callback.lock().await;
                    invoke_callback(
                        &mut *cb_guard,
                        PutEvent::ReservationProgress {
                            current: successful_pad_count as u64,
                            total: num_pads_to_reserve as u64,
                        },
                    )
                    .await?;
                }
            }
            Err(e) => {
                // A reservation task failed
                error!(
                    "Reservation[{}]: Failed to reserve a pad: {}. Aborting remaining reservations.",
                    key_for_log, e
                );
                // Send a generic error signal via channel so writer knows to stop
                let signal_err = Error::AllocationFailed("Pad reservation failed".to_string());
                pad_info_tx.send(Err(signal_err)).await.ok(); // Ignore send error
                                                              // Drop the stream to cancel pending futures
                drop(stream);
                // Return the original error
                return Err(e);
            }
        }
    }

    // If we reach here, all pads were reserved successfully
    info!(
        "Reservation[{}]: Successfully reserved all {} pads.",
        key_for_log, num_pads_to_reserve
    );
    Ok(())
}
