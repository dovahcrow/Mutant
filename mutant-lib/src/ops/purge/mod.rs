use crate::error::Error;
use crate::events::{PurgeCallback, PurgeEvent};
use crate::index::master_index::MasterIndex;
use crate::index::PadInfo;
use crate::internal_events::invoke_purge_callback;
use crate::network::client::{ClientManager, Config};
use crate::network::{Network, NetworkError};
use crate::ops::worker::{AsyncTask, PoolError, WorkerPoolConfig};
use ant_networking::GetRecordError;
use async_trait::async_trait;
use deadpool::managed::Object;
use log::{debug, error, info, warn};
use mutant_protocol::PurgeResult;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    processed_items_counter: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Clone)]
struct PurgeTaskProcessor {
    context: Arc<PurgeContext>,
}

impl PurgeTaskProcessor {
    fn new(context: Arc<PurgeContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl AsyncTask<PadInfo, PurgeContext, Object<ClientManager>, PurgeTaskOutcome, Error>
    for PurgeTaskProcessor
{
    type ItemId = ();

    async fn process(
        &self,
        worker_id: usize,
        client: &Object<ClientManager>,
        pad: PadInfo,
    ) -> Result<(Self::ItemId, PurgeTaskOutcome), (Error, PadInfo)> {
        let get_result = self.context.network.get(client, &pad.address, None).await;

        let outcome: PurgeTaskOutcome;

        match get_result {
            Ok(res) => {
                debug!("Worker {} verified pad {}.", worker_id, pad.address);
                match self.context.index.try_write() {
                    Ok(mut index_guard) => {
                        let mut pad = pad.clone();
                        pad.last_known_counter = res.counter;
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
                    match self.context.index.try_write() {
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
                    match self.context.index.try_write() {
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
                    if self.context.aggressive {
                        debug!(
                            "Worker {} discarding pad {} (aggressive due to error: {}).",
                            worker_id, pad.address, e
                        );
                        match self.context.index.try_write() {
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

        invoke_purge_callback(&self.context.purge_callback, PurgeEvent::PadProcessed)
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

        self.context
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

    let processed_items_counter_atomic = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let purge_context = Arc::new(PurgeContext {
        index: index.clone(),
        network: network.clone(),
        aggressive,
        purge_callback: callback.clone(),
        processed_items_counter: processed_items_counter_atomic.clone(),
    });

    let task_processor = PurgeTaskProcessor::new(purge_context.clone());

    let config = WorkerPoolConfig {
        network: network.clone(),
        client_config: Config::Get,
        task_processor,
        enable_recycling: false,
        total_items_hint: total_pads,
    };

    let (pool, handles) = match crate::ops::worker::build(config).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to build worker pool for purge: {:?}", e);
            return match e {
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
                _ => Err(Error::Internal(format!("Pool build failed: {:?}", e))),
            };
        }
    };

    let send_pads_task = {
        let pads_clone = pads;
        let worker_txs = handles.worker_txs;

        tokio::spawn(async move {
            let mut worker_index = 0;
            let num_workers = worker_txs.len();
            if num_workers == 0 {
                warn!("Purge distribution task: No workers to distribute to!");
                return;
            }

            debug!(
                "Purge distributing {} pads to {} workers...",
                pads_clone.len(),
                num_workers
            );

            for pad in pads_clone {
                let target_tx = &worker_txs[worker_index % num_workers];
                if target_tx.send(pad).await.is_err() {
                    warn!("Purge distribution failed: Worker channel closed unexpectedly.");
                    break;
                }
                worker_index += 1;
            }

            debug!("Purge distribution finished, closing worker channels.");
        })
    };

    let pool_run_result = pool.run().await;

    if let Err(e) = send_pads_task.await {
        error!("Purge distribution task panicked: {:?}", e);
        return Err(Error::Internal(format!(
            "Pad distribution task failed: {:?}",
            e
        )));
    }
    debug!("Purge distribution task completed.");

    match pool_run_result {
        Ok(purge_outcomes) => {
            debug!(
                "Purge pool completed. Processing {} outcomes.",
                purge_outcomes.len()
            );

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
            error!("Purge worker pool failed: {:?}", pool_error);
            match pool_error {
                PoolError::TaskError(task_err) => Err(task_err),
                PoolError::JoinError(join_err) => Err(Error::Internal(format!(
                    "Worker task join error: {:?}",
                    join_err
                ))),
                PoolError::PoolSetupError(msg) => {
                    Err(Error::Internal(format!("Pool setup error: {}", msg)))
                }
                PoolError::ClientAcquisitionError(msg) => {
                    Err(Error::Network(NetworkError::ClientAccessError(msg)))
                }
            }
        }
    }
}
