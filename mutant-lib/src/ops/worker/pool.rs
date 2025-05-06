use async_channel::{Receiver, Sender};
use deadpool::managed::Object;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, error, info, warn};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::Error as MutantError;
use crate::network::client::ClientManager;
use super::async_task::AsyncTask;
use super::error::PoolError;
use super::worker::Worker;

pub struct WorkerPool<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    pub(crate) task: Arc<Task>,
    pub(crate) clients: Vec<Arc<Client>>,
    // Channels managed internally by the pool
    pub(crate) worker_txs: Vec<Sender<Item>>,
    pub(crate) worker_rxs: Vec<Receiver<Item>>,
    pub(crate) global_tx: Sender<Item>,
    pub(crate) global_rx: Receiver<Item>,
    pub(crate) retry_sender: Option<Sender<(E, Item)>>,
    pub(crate) retry_rx: Option<Receiver<(E, Item)>>,
    pub(crate) _marker_context: PhantomData<Context>,
    pub(crate) _marker_result: PhantomData<T>,
    pub(crate) _marker_error: PhantomData<E>,
}

impl<Item, Context, Task, T, E> WorkerPool<Item, Context, Object<ClientManager>, Task, T, E>
where
    Item: Send + 'static + Debug,
    Context: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Object<ClientManager>, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: Debug + Send + Clone + 'static + From<MutantError>,
{
    pub async fn send_items(&self, items: Vec<Item>) -> Result<(), PoolError<E>> {
        let num_workers = self.worker_txs.len();
        if num_workers == 0 {
            warn!("WorkerPool::send_items: No workers to distribute to!");
            return Ok(());
        }

        let item_count = items.len();
        debug!(
            "WorkerPool distributing {} items to {} workers...",
            item_count,
            num_workers
        );

        let mut worker_index = 0;
        for item in items {
            let target_tx = &self.worker_txs[worker_index % num_workers];
            if target_tx.send(item).await.is_err() {
                let err_msg = format!(
                    "WorkerPool::send_items failed: Worker channel {} closed unexpectedly.",
                    worker_index % num_workers
                );
                error!("{}", err_msg);
                return Err(PoolError::PoolSetupError(err_msg));
            }
            worker_index += 1;
        }

        // Note: We don't close the channels here anymore.
        // The channels will be closed in the run method after workers have processed the items.
        debug!("WorkerPool distribution finished, sent {} items to workers.", item_count);
        Ok(())
    }

    pub async fn run(
        mut self,
        recycle_fn: Option<
            Arc<
                dyn Fn(
                        E,
                        Item,
                    ) -> futures::future::BoxFuture<
                        'static,
                        Result<Option<Item>, MutantError>,
                    > + Send
                    + Sync,
            >,
        >,
    ) -> Result<Vec<(Task::ItemId, T)>, PoolError<E>> {
        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

        let worker_rxs = std::mem::take(&mut self.worker_rxs);
        let maybe_retry_rx = self.retry_rx.take();

        let task = self.task.clone();
        let clients = self.clients.clone();
        let retry_sender_clone = self.retry_sender.clone();

        // Keep a clone of the global_tx for the recycler
        let global_tx_for_recycler = self.global_tx.clone();

        // Create a vector to store all global_rx clones that we'll need to close later
        let mut global_rx_clones = Vec::new();

        let mut worker_handles = FuturesUnordered::new();

        for ((worker_id, worker_rx), client) in
            worker_rxs.into_iter().enumerate().zip(clients.into_iter())
        {
            // Clone the global_rx for this worker and keep track of it
            let worker_global_rx = self.global_rx.clone();
            global_rx_clones.push(worker_global_rx.clone());

            let worker: Worker<Item, Context, Object<ClientManager>, Task, T, E> = Worker {
                id: worker_id,
                client,
                task_processor: task.clone(),
                local_queue: worker_rx,
                global_queue: worker_global_rx,
                retry_sender: retry_sender_clone.clone(),
                results_collector: results_collector.clone(),
                errors_collector: errors_collector.clone(),
                _marker_context: PhantomData,
            };
            worker_handles.push(tokio::spawn(worker.run()));
        }

        // Drop the original global_rx
        drop(self.global_rx);

        let recycler_handle = if let (Some(retry_rx), Some(recycle_function)) =
            (maybe_retry_rx.clone(), recycle_fn)
        {
            Some(tokio::spawn(async move {
                debug!("WorkerPool internal recycler task started. Processing recycling queue...");

                let mut recycled_count = 0;
                let mut dropped_count = 0;
                let mut error_count = 0;

                // Log that we're starting the recycling process
                info!("Starting recycling process for failed items...");

                // Process the recycling queue until it's empty
                // This ensures all failed pads are properly recycled
                info!("Starting recycling loop to process all failed pads...");
                'recycling_loop: loop {
                    match retry_rx.recv().await {
                        Ok((error_cause, item_to_recycle)) => {
                            debug!(
                                "Recycler received item {:?} due to error: {:?}",
                                item_to_recycle, error_cause
                            );
                            match recycle_function(error_cause, item_to_recycle).await {
                                Ok(Some(new_item)) => {
                                    recycled_count += 1;
                                    debug!(
                                        "Recycled into new item {:?}, sending back to pool. (Total recycled: {})",
                                        new_item, recycled_count
                                    );
                                    if global_tx_for_recycler.send(new_item).await.is_err() {
                                        error!("Recycler failed to send recycled item to global channel. Pool might be closed.");
                                        break 'recycling_loop;
                                    }
                                }
                                Ok(None) => {
                                    dropped_count += 1;
                                    debug!("Recycling resulted in no new item. Dropping. (Total dropped: {})", dropped_count);
                                }
                                Err(recycle_err) => {
                                    error_count += 1;
                                    error!("Recycling failed: {:?}. Skipping item. (Total errors: {})", recycle_err, error_count);
                                }
                            }
                        },
                        Err(e) => {
                            debug!("Recycling channel closed or empty: {}. Exiting recycling loop.", e);
                            // The channel is closed, which means no more items to recycle
                            break 'recycling_loop;
                        }
                    }
                }
                debug!("Recycle channel closed. Recycler task finishing. Stats: recycled={}, dropped={}, errors={}",
                      recycled_count, dropped_count, error_count);

                Ok::<(), PoolError<E>>(())
            }))
        } else {
            debug!("Recycling not enabled or recycle function not provided.");
            if let Some(rx) = maybe_retry_rx {
                Self::spawn_drainer(rx);
            }
            None
        };

        // First, close all channels to signal workers to finish
        debug!("Closing all channels to signal workers to finish...");

        // Close the worker channels
        for tx in self.worker_txs.iter() {
            tx.close();
        }
        debug!("Closed worker channels.");

        // Close the global_tx to prevent new items from being added
        self.global_tx.close();
        debug!("Closed global channel.");

        // Close all global_rx clones to ensure workers can terminate
        for rx in &global_rx_clones {
            rx.close();
        }
        debug!("Closed all global_rx clones.");

        // Now close the retry channel if it exists
        if let Some(sender) = &self.retry_sender {
            sender.close();
            debug!("Closed retry channel.");
        }

        // Wait for all workers to complete
        debug!("Waiting for {} workers to complete...", worker_handles.len());

        let mut completed_workers = 0;
        let total_workers = worker_handles.len();

        // Wait for all workers to complete
        while !worker_handles.is_empty() {
            match worker_handles.next().await {
                Some(result) => {
                    match result {
                        Ok(Ok(())) => {
                            completed_workers += 1;
                            debug!("Worker completed successfully ({}/{})", completed_workers, total_workers);
                        }
                        Ok(Err(e)) => {
                            error!("Worker failed: {:?}", e);
                            return Err(e);
                        }
                        Err(join_err) => {
                            error!("Worker panicked: {:?}", join_err);
                            return Err(PoolError::JoinError(join_err));
                        }
                    }
                },
                None => {
                    // No more workers to wait for
                    debug!("No more workers to wait for.");
                    break;
                }
            }
        }

        debug!("All {} workers completed.", total_workers);

        // Wait for recycler to complete if it exists
        if let Some(handle) = recycler_handle {
            debug!("Waiting for recycler task to complete...");

            match handle.await {
                Ok(Ok(())) => {
                    debug!("Internal recycler task completed successfully.");
                }
                Ok(Err(recycler_pool_error)) => {
                    error!("Internal recycler task failed: {:?}", recycler_pool_error);
                    // Return the error to ensure the caller knows recycling failed
                    return Err(recycler_pool_error);
                }
                Err(join_err) => {
                    error!("Internal recycler task panicked: {:?}", join_err);
                    return Err(PoolError::JoinError(join_err));
                }
            }
        } else {
            debug!("No recycler task to wait for.");
        }

        // Check for errors
        let final_errors = match errors_collector.try_lock() {
            Ok(errors) => errors,
            Err(_) => {
                warn!("Could not immediately acquire lock on errors collector. Waiting...");
                errors_collector.lock().await
            }
        };

        if !final_errors.is_empty() {
            return Err(PoolError::TaskError(final_errors.first().unwrap().clone()));
        }

        // Drop the errors lock before trying to get results
        drop(final_errors);

        // Try to get the results with a timeout
        let max_attempts = 5;
        let mut attempt = 0;

        loop {
            attempt += 1;
            debug!("Attempting to collect results (attempt {}/{})", attempt, max_attempts);

            // First try to lock the results collector
            match results_collector.try_lock() {
                Ok(results) => {
                    debug!("Successfully locked results collector");
                    let results_copy = results.clone(); // Make a copy while we have the lock
                    drop(results); // Release the lock

                    // Now try to unwrap the Arc
                    match Arc::try_unwrap(results_collector.clone()) {
                        Ok(mutex) => {
                            let final_results = mutex.into_inner();
                            debug!("WorkerPool completed successfully with {} results.", final_results.len());
                            return Ok(final_results);
                        },
                        Err(_) => {
                            // Arc still has other references, return our copy
                            debug!("Could not unwrap results collector Arc, but we have a copy with {} results.", results_copy.len());
                            return Ok(results_copy);
                        }
                    }
                },
                Err(_) => {
                    warn!("Could not lock results collector on attempt {}/{}. Waiting...", attempt, max_attempts);

                    if attempt >= max_attempts {
                        error!("Failed to collect results after {} attempts", max_attempts);
                        return Err(PoolError::PoolSetupError(
                            format!("Failed to collect results after {} attempts", max_attempts)
                        ));
                    }

                    continue;
                }
            };
        }
    }

    fn spawn_drainer(rx: Receiver<(E, Item)>) {
        tokio::spawn(async move {
            // Don't close the channel immediately, let it drain first
            while let Ok((err, item)) = rx.recv().await {
                warn!(
                    "Draining item {:?} from unused/unconfigured retry queue due to error: {:?}",
                    item, err
                );
            }
            debug!("Finished draining retry queue.");
        });
    }
}
