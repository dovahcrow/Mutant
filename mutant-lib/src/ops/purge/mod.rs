use crate::error::Error;
use crate::events::{PurgeCallback, PurgeEvent};
use crate::index::master_index::MasterIndex;
use crate::index::PadInfo;
use crate::internal_events::invoke_purge_callback;
use crate::network::client::{ClientManager, Config};
use crate::network::{Network, NetworkError};
use crate::ops::worker::{AsyncTask, PoolError, WorkerPool};
use ant_networking::GetRecordError;
use async_channel::bounded;
use async_trait::async_trait;
use deadpool::managed::Object;
use log::{debug, error, info, warn};
use mutant_protocol::PurgeResult;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

use super::{BATCH_SIZE, WORKER_COUNT};

#[derive(Debug, Clone, Copy)]
enum PurgeTaskOutcome {
    Verified,
    VerifiedNotEnoughCopies,
    DiscardedNotFound,
    DiscardedAggressive,
    KeptNonAggressiveError,
}

#[derive(Clone)]
struct PurgeContext {
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    aggressive: bool,
    purge_callback: Option<PurgeCallback>,
    client_manager: Arc<Object<ClientManager>>,
    processed_items_counter: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Clone)]
struct PurgeTaskProcessor;

#[async_trait]
impl AsyncTask<PadInfo, PurgeContext, PurgeTaskOutcome, Error> for PurgeTaskProcessor {
    type ItemId = ();

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<PurgeContext>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, PurgeTaskOutcome), (Error, PadInfo)> {
        let client = &*context.client_manager;

        let get_result = context.network.get(client, &pad.address, None).await;

        let outcome: PurgeTaskOutcome;

        match get_result {
            Ok(_res) => {
                debug!("Worker {} verified pad {}.", worker_id, pad.address);
                match context.index.try_write() {
                    Ok(mut index_guard) => {
                        index_guard
                            .verified_pending_pad(pad.clone())
                            .map_err(|e| (e.into(), pad.clone()))?;
                        outcome = PurgeTaskOutcome::Verified;
                    }
                    Err(_) => {
                        error!(
                            "Worker {} could not acquire index write lock for verifying pad {}",
                            worker_id, pad.address
                        );
                        return Err((
                            Error::Internal(format!(
                                "Index lock failed for verify {}",
                                pad.address
                            )),
                            pad,
                        ));
                    }
                }
            }
            Err(e) => match e {
                NetworkError::GetError(GetRecordError::RecordNotFound) => {
                    debug!(
                        "Worker {} discarding pad {} (NotFound).",
                        worker_id, pad.address
                    );
                    match context.index.try_write() {
                        Ok(mut index_guard) => {
                            index_guard
                                .discard_pending_pad(pad.clone())
                                .map_err(|e| (e.into(), pad.clone()))?;
                            outcome = PurgeTaskOutcome::DiscardedNotFound;
                        }
                        Err(_) => {
                            error!("Worker {} could not acquire index write lock for discarding pad {} (NotFound)", worker_id, pad.address);
                            return Err((
                                Error::Internal(format!(
                                    "Index lock failed for discard (NotFound) {}",
                                    pad.address
                                )),
                                pad,
                            ));
                        }
                    }
                }
                NetworkError::GetError(GetRecordError::NotEnoughCopies { .. }) => {
                    debug!(
                        "Worker {} verified pad {} but not enough copies reported by node.",
                        worker_id, pad.address
                    );
                    match context.index.try_write() {
                        Ok(mut index_guard) => {
                            index_guard
                                .verified_pending_pad(pad.clone())
                                .map_err(|e| (e.into(), pad.clone()))?;
                            outcome = PurgeTaskOutcome::VerifiedNotEnoughCopies;
                        }
                        Err(_) => {
                            error!("Worker {} could not acquire index write lock for verifying pad {} (NotEnoughCopies)", worker_id, pad.address);
                            return Err((
                                Error::Internal(format!(
                                    "Index lock failed for verify (NotEnoughCopies) {}",
                                    pad.address
                                )),
                                pad,
                            ));
                        }
                    }
                }
                _ => {
                    if context.aggressive {
                        debug!(
                            "Worker {} discarding pad {} (aggressive due to error: {}).",
                            worker_id, pad.address, e
                        );
                        match context.index.try_write() {
                            Ok(mut index_guard) => {
                                index_guard
                                    .discard_pending_pad(pad.clone())
                                    .map_err(|e| (e.into(), pad.clone()))?;
                                outcome = PurgeTaskOutcome::DiscardedAggressive;
                            }
                            Err(_) => {
                                error!("Worker {} could not acquire index write lock for discarding pad {} (aggressive)", worker_id, pad.address);
                                return Err((
                                    Error::Internal(format!(
                                        "Index lock failed for discard (aggressive) {}",
                                        pad.address
                                    )),
                                    pad,
                                ));
                            }
                        }
                    } else {
                        warn!("Worker {} found pad {} but encountered an error ({}); keeping in pending (non-aggressive).", worker_id, pad.address, e);
                        outcome = PurgeTaskOutcome::KeptNonAggressiveError;
                    }
                }
            },
        }

        invoke_purge_callback(&context.purge_callback, PurgeEvent::PadProcessed)
            .await
            .map_err(|cb_err| {
                error!(
                    "Worker {} callback error (PadProcessed): {:?}",
                    worker_id, cb_err
                );
                (
                    Error::Internal(format!("Callback error: {:?}", cb_err)),
                    pad.clone(),
                )
            })?;

        context
            .processed_items_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(((), outcome))
    }
}

pub(super) async fn purge(
    index: Arc<RwLock<MasterIndex>>,
    network: Arc<Network>,
    aggressive: bool,
    purge_callback: Option<PurgeCallback>,
) -> Result<PurgeResult, Error> {
    let pads = index.read().await.get_pending_pads();
    let total_pads = pads.len();
    let callback = purge_callback.clone();

    invoke_purge_callback(
        &callback,
        PurgeEvent::Starting {
            total_count: total_pads,
        },
    )
    .await
    .map_err(|e| Error::Internal(format!("Starting callback failed: {:?}", e)))?;

    if total_pads == 0 {
        info!("No pending pads to purge.");
        invoke_purge_callback(
            &callback,
            PurgeEvent::Complete {
                verified_count: 0,
                failed_count: 0,
            },
        )
        .await
        .map_err(|e| Error::Internal(format!("Complete callback failed (no pads): {:?}", e)))?;
        return Ok(PurgeResult { nb_pads_purged: 0 });
    }

    // Create worker-specific and global channels
    let mut worker_txs = Vec::with_capacity(WORKER_COUNT);
    let mut worker_rxs = Vec::with_capacity(WORKER_COUNT);
    for _ in 0..WORKER_COUNT {
        let (tx, rx) = bounded::<PadInfo>(total_pads.saturating_add(1) / WORKER_COUNT + BATCH_SIZE);
        worker_txs.push(tx);
        worker_rxs.push(rx);
    }
    let (global_tx, global_rx) = bounded::<PadInfo>(total_pads + WORKER_COUNT * BATCH_SIZE); // Generous buffer

    let client_guard = Arc::new(
        network
            .get_client(Config::Get)
            .await
            .map_err(|e| Error::Network(NetworkError::ClientAccessError(e.to_string())))?,
    );

    let completion_notifier = Arc::new(Notify::new());
    let total_items_atomic = Arc::new(std::sync::atomic::AtomicUsize::new(total_pads));
    let processed_items_counter_atomic = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let purge_context = Arc::new(PurgeContext {
        index: index.clone(),
        network: network.clone(),
        aggressive,
        purge_callback: callback.clone(),
        client_manager: client_guard,
        processed_items_counter: processed_items_counter_atomic.clone(),
    });

    let send_pads_task = {
        let pads_clone = pads;
        let worker_txs_clone = worker_txs.clone();
        let global_tx_clone = global_tx.clone(); // To close later

        tokio::spawn(async move {
            let mut worker_index = 0;
            for pad in pads_clone {
                // Send round-robin
                let target_tx = &worker_txs_clone[worker_index % WORKER_COUNT];
                if target_tx.send(pad).await.is_err() {
                    // error!("Failed to send pad to PURGE worker {} channel, receiver likely closed.", worker_index % WORKER_COUNT);
                    break;
                }
                worker_index += 1;
            }
            // Close worker channels
            for tx in worker_txs_clone {
                tx.close();
            }
            // Close global channel
            global_tx_clone.close();
        })
    };

    let pool = WorkerPool::new(
        WORKER_COUNT,
        BATCH_SIZE,
        purge_context.clone(),
        Arc::new(PurgeTaskProcessor),
        worker_rxs, // Pass worker receivers
        global_rx,  // Pass global receiver
        None,       // No retry channel needed for purge
        completion_notifier.clone(),
        Some(total_items_atomic.clone()),
        Some(processed_items_counter_atomic.clone()),
    );

    // info!(
    //     "Starting purge worker pool with {} workers for {} pads.",
    //     WORKER_COUNT, total_pads
    // );

    let pool_run_result = pool.run().await;

    if let Err(_e) = send_pads_task.await {
        // error!("Send pads task panicked: {:?}", e);
    } else {
        // debug!("Send pads task completed successfully.");
    }

    match pool_run_result {
        Ok(purge_outcomes) => {
            // info!(
            //     "Purge worker pool finished. Processed outcomes for {} pads.",
            //     purge_outcomes.len()
            // );

            if purge_outcomes.len() != total_pads {
                // warn!(
                //      "Purge pool finished OK, but outcome count ({}) doesn't match total pads ({}). This might indicate an issue with worker error handling or counters.",
                //      purge_outcomes.len(),
                //      total_pads
                //  );
            }

            let mut verified_count = 0;
            let mut discarded_count = 0;

            for (_, outcome) in purge_outcomes {
                match outcome {
                    PurgeTaskOutcome::Verified | PurgeTaskOutcome::VerifiedNotEnoughCopies => {
                        verified_count += 1;
                    }
                    PurgeTaskOutcome::DiscardedNotFound | PurgeTaskOutcome::DiscardedAggressive => {
                        discarded_count += 1;
                    }
                    PurgeTaskOutcome::KeptNonAggressiveError => {}
                }
            }

            invoke_purge_callback(
                &callback,
                PurgeEvent::Complete {
                    verified_count,
                    failed_count: discarded_count,
                },
            )
            .await
            .map_err(|e| Error::Internal(format!("Complete callback failed: {:?}", e)))?;

            Ok(PurgeResult {
                nb_pads_purged: discarded_count,
            })
        }
        Err(pool_error) => {
            // error!("Purge worker pool failed: {:?}", pool_error);
            match pool_error {
                PoolError::TaskError(task_err) => Err(task_err),
                PoolError::JoinError(join_err) => Err(Error::Internal(format!(
                    "Worker task join error: {:?}",
                    join_err
                ))),
                PoolError::PoolSetupError(msg) => {
                    Err(Error::Internal(format!("Pool setup error: {}", msg)))
                }
                PoolError::SemaphoreClosed => Err(Error::Internal(
                    "Worker semaphore closed unexpectedly".to_string(),
                )),
            }
        }
    }
}
