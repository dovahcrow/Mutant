use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::storage::Storage;
use crate::utils::retry::retry_operation;
use futures::stream::{self, StreamExt};
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) const MAX_CONCURRENT_RESERVATIONS: usize = 10;

/// Reserves a specified number of new pads concurrently and collects successful results.
/// It sends progress updates via the callback.
pub(super) async fn reserve_new_pads_and_collect(
    key_for_log: &str,
    num_pads_to_reserve: usize,
    shared_callback: Arc<Mutex<Option<PutCallback>>>,
    collector: Arc<Mutex<Vec<PadInfo>>>,
    storage: Arc<Storage>,
) -> Result<(), Error> {
    info!(
        "Reservation[{}]: Reserving {} new pads (max concurrent: {})...",
        key_for_log, num_pads_to_reserve, MAX_CONCURRENT_RESERVATIONS
    );

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

    let results = stream::iter(0..num_pads_to_reserve)
        .map(|i| {
            let storage_clone = storage.clone();
            let key_clone = key_for_log.to_string();
            let cb = shared_callback.clone();
            let coll = collector.clone();

            async move {
                let pad_index = i;

                let reserve_result = retry_operation(
                    &format!("Reserve Pad #{}", pad_index),
                    || async {
                        let (pad_address, pad_key) =
                            storage_clone.create_scratchpad_internal_raw(&[], 0).await?;
                        Ok((pad_address, pad_key.to_bytes().to_vec()))
                    },
                    |_e: &Error| true,
                )
                .await;

                {
                    let mut cb_guard = cb.lock().await;
                    invoke_callback(
                        &mut *cb_guard,
                        PutEvent::PadReserved {
                            index: pad_index as u64,
                            total: num_pads_to_reserve as u64,
                            result: reserve_result
                                .as_ref()
                                .map(|_| ())
                                .map_err(|e| e.to_string()),
                        },
                    )
                    .await
                    .ok();
                }

                match reserve_result {
                    Ok(pad_info) => {
                        debug!(
                            "Reservation[{}]: Pad #{} reserved successfully: {:?}",
                            key_clone, pad_index, pad_info.0
                        );
                        {
                            let mut coll_guard = coll.lock().await;
                            coll_guard.push(pad_info.clone());
                        }
                        Ok(pad_info)
                    }
                    Err(e) => {
                        error!(
                            "Reservation[{}]: Failed to reserve pad #{}: {}",
                            key_clone, pad_index, e
                        );
                        Err(e)
                    }
                }
            }
        })
        .buffer_unordered(MAX_CONCURRENT_RESERVATIONS)
        .collect::<Vec<Result<PadInfo, Error>>>()
        .await;

    let mut successful_pads = Vec::new();
    for result in results {
        match result {
            Ok(pad_info) => successful_pads.push(pad_info),
            Err(e) => {
                {
                    let mut coll_guard = collector.lock().await;
                    coll_guard.clear();
                    warn!(
                        "Reservation[{}]: Failed. Collector cleared due to error: {}",
                        key_for_log, e
                    );
                }
                return Err(e);
            }
        }
    }

    let collected_count = {
        let coll_guard = collector.lock().await;
        coll_guard.len()
    };

    if collected_count == num_pads_to_reserve {
        info!(
            "Reservation[{}]: Successfully reserved and collected all {} required pads.",
            key_for_log, num_pads_to_reserve
        );
        Ok(())
    } else {
        error!(
            "Reservation[{}]: Logic error: Collected {} pads, but expected {}. Clearing collector.",
            key_for_log, collected_count, num_pads_to_reserve
        );
        {
            let mut coll_guard = collector.lock().await;
            coll_guard.clear();
        }
        Err(Error::InternalError(
            "Pad reservation count mismatch after collection".to_string(),
        ))
    }
}
