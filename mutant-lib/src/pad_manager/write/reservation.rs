use super::super::PadManager;
use super::PadInfo;
use crate::error::Error;
use crate::events::{invoke_callback, PutCallback, PutEvent};
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
    pub(super) async fn reserve_new_pads(
        &self,
        num_pads_to_reserve: usize,
        callback_arc: Arc<Mutex<Option<PutCallback>>>,
    ) -> Result<(mpsc::Receiver<Result<PadInfo, Error>>, JoinHandle<()>), Error> {
        if num_pads_to_reserve == 0 {
            let (tx, rx) = mpsc::channel(1);
            drop(tx);
            let handle = tokio::spawn(async {});
            return Ok((rx, handle));
        }

        let (pad_sender, pad_receiver) = mpsc::channel(num_pads_to_reserve);

        let storage_clone = self.storage.clone();
        let semaphore = Arc::new(Semaphore::new(WRITE_TASK_CONCURRENCY));

        let join_handle = tokio::spawn(async move {
            let mut join_set = JoinSet::new();

            for i in 0..num_pads_to_reserve {
                let task_storage_clone = storage_clone.clone();
                let task_semaphore_clone = semaphore.clone();
                let task_pad_sender = pad_sender.clone();

                join_set.spawn(async move {
                    let permit = task_semaphore_clone
                        .acquire_owned()
                        .await
                        .expect("Semaphore closed");

                    let res_result = retry_operation(
                        &format!("Reserve scratchpad {}/{}", i + 1, num_pads_to_reserve),
                        || task_storage_clone.create_scratchpad_internal_raw(&[0], 0),
                        |_e: &Error| true,
                    )
                    .await;
                    drop(permit);

                    let send_result = match res_result {
                        Ok((address, secret_key)) => {
                            let key_bytes = secret_key.to_bytes().to_vec();
                            let pad_info: PadInfo = (address, key_bytes);
                            task_pad_sender.send(Ok(pad_info)).await
                        }
                        Err(e) => {
                            error!("Reservation Sub-Task Failed (idx {}): {}", i, e);
                            task_pad_sender.send(Err(e)).await
                        }
                    };

                    if let Err(send_err) = send_result {
                        error!(
                            "Failed to send reservation result for index {} to channel: {}",
                            i, send_err
                        );
                    }
                });
            }

            drop(pad_sender);

            let mut successful_count = 0u64;
            let mut first_error: Option<Error> = None;
            let total_to_reserve = num_pads_to_reserve as u64;

            while let Some(join_res) = join_set.join_next().await {
                match join_res {
                    Ok(_) => {
                        successful_count += 1;
                        let callback_clone = callback_arc.clone();
                        tokio::spawn(async move {
                            let mut cb_guard = callback_clone.lock().await;
                            if let Err(e) = invoke_callback(
                                &mut *cb_guard,
                                PutEvent::ReservationProgress {
                                    current: successful_count,
                                    total: total_to_reserve,
                                },
                            )
                            .await
                            {
                                error!("Error invoking ReservationProgress callback: {}", e);
                            }
                        });
                    }
                    Err(join_error) => {
                        error!("Reservation Task: JoinError: {}", join_error);
                        if first_error.is_none() {
                            first_error = Some(Error::from_join_error_msg(
                                &join_error,
                                "Reservation sub-task join error".to_string(),
                            ));
                        }
                    }
                }
            }
            if let Some(e) = first_error {
                warn!(
                    "Reservation background task completed with join errors: {}",
                    e
                );
            } else {
                debug!("Reservation background task completed.");
            }
        });

        Ok((pad_receiver, join_handle))
    }
}
