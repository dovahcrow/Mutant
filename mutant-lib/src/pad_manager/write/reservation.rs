use super::super::PadManager;
use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
use crate::storage::Storage;
use crate::utils::retry::retry_operation;
use log::{debug, error, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::task::JoinSet;

const WRITE_TASK_CONCURRENCY: usize = 20;

impl PadManager {
    /// Reserves the specified number of new scratchpads concurrently.
    /// Awaits completion and returns all successfully reserved pads or the first error.
    pub(super) async fn reserve_new_pads_and_collect(
        &self,
        num_pads_to_reserve: usize,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
    ) -> Result<Vec<PadInfo>, Error> {
        if num_pads_to_reserve == 0 {
            debug!("reserve_new_pads_and_collect: No pads requested, returning empty vec.");
            return Ok(Vec::new());
        }

        debug!(
            "reserve_new_pads_and_collect: Reserving {} pads...",
            num_pads_to_reserve
        );
        let mut join_set = JoinSet::new();
        let semaphore = Arc::new(Semaphore::new(WRITE_TASK_CONCURRENCY));
        let mut reserved_pads = Vec::with_capacity(num_pads_to_reserve);
        let mut first_error: Option<Error> = None;

        {
            let mut cb_guard = callback_arc.lock().await;
            invoke_callback(
                &mut *cb_guard,
                PutEvent::ReservingPads {
                    count: num_pads_to_reserve as u64,
                },
            )
            .await?;
        }

        for i in 0..num_pads_to_reserve {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    let err = Error::InternalError(
                        "Reservation semaphore closed unexpectedly".to_string(),
                    );
                    error!("reserve_new_pads_and_collect: {}", err);
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                    break;
                }
            };

            let task_storage = self.storage.clone();
            let task_callback = callback_arc.clone();

            join_set.spawn(async move {
                let reservation_result = retry_operation(
                    &format!("Reserve Pad #{}", i),
                    || async {
                        let (pad_address, pad_key) =
                            task_storage.create_scratchpad_internal_raw(&[], 0).await?;
                        Ok((pad_address, pad_key.to_bytes().to_vec()))
                    },
                    |_e: &Error| true,
                )
                .await;

                let mut cb_guard = task_callback.lock().await;
                invoke_callback(
                    &mut *cb_guard,
                    PutEvent::PadReserved {
                        index: i as u64,
                        total: num_pads_to_reserve as u64,
                        result: reservation_result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|e| e.to_string()),
                    },
                )
                .await
                .ok();

                drop(permit);
                reservation_result
            });
        }

        debug!(
            "reserve_new_pads_and_collect: All {} reservation tasks spawned. Waiting for completion...",
            join_set.len()
        );

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok(task_result) => match task_result {
                    Ok(pad_info) => {
                        reserved_pads.push(pad_info);
                    }
                    Err(e) => {
                        error!(
                            "reserve_new_pads_and_collect: Reservation sub-task failed: {}",
                            e
                        );
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                },
                Err(join_error) => {
                    error!(
                        "reserve_new_pads_and_collect: JoinError during reservation: {}",
                        join_error
                    );
                    if first_error.is_none() {
                        first_error = Some(Error::from_join_error_msg(
                            &join_error,
                            "Reservation sub-task join error".to_string(),
                        ));
                    }
                }
            }
        }
        debug!(
            "reserve_new_pads_and_collect: All reservation tasks finished. Collected {} pads.",
            reserved_pads.len()
        );

        if let Some(err) = first_error {
            error!(
                "reserve_new_pads_and_collect: Returning first error encountered: {}",
                err
            );
            Err(err)
        } else if reserved_pads.len() != num_pads_to_reserve {
            let err = Error::InternalError(format!(
                "Reservation count mismatch: Expected {}, got {}",
                num_pads_to_reserve,
                reserved_pads.len()
            ));
            error!("reserve_new_pads_and_collect: {}", err);
            Err(err)
        } else {
            debug!(
                "reserve_new_pads_and_collect: Successfully reserved all {} pads.",
                reserved_pads.len()
            );
            Ok(reserved_pads)
        }
    }

    // Remove the commented out old function
    /*
    pub(super) async fn reserve_new_pads(
        &self,
        num_pads_to_reserve: usize,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
    ) -> Result<(mpsc::Receiver<Result<PadInfo, Error>>, JoinHandle<()>), Error> {
       // ... old implementation ...
    }
    */
}
