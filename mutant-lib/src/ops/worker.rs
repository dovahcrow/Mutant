use crate::network::client::ClientManager;
use crate::network::client::Config as ClientConfig;
use crate::network::Network;
use async_channel::{bounded, Receiver, Sender};
use deadpool::managed::Object;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, error, trace, warn};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::network::BATCH_SIZE;
use crate::network::NB_CLIENTS;

// Trait for the actual work function
#[async_trait::async_trait]
pub trait AsyncTask<Item, Context, Client, TaskResult, TaskError>: Send + Sync + 'static
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    TaskResult: Send + 'static,
    TaskError: Debug + Send + 'static,
{
    type ItemId: Send + Debug + Clone;

    async fn process(
        &self,
        worker_id: usize,
        client: &Client,
        item: Item,
    ) -> Result<(Self::ItemId, TaskResult), (TaskError, Item)>;
}

#[derive(Debug)]
pub enum PoolError<TaskError>
where
    TaskError: Debug,
{
    TaskError(TaskError),
    JoinError(tokio::task::JoinError),
    PoolSetupError(String),
    ClientAcquisitionError(String),
}

// Step 4: Define WorkerPoolConfig Struct
pub struct WorkerPoolConfig<Task> {
    pub network: Arc<Network>,
    pub client_config: ClientConfig,
    pub task_processor: Task,
    pub enable_recycling: bool,
    pub total_items_hint: usize,
}

// Step 2: Modify Worker Struct
struct Worker<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    id: usize,
    client: Arc<Client>,
    task_processor: Arc<Task>,
    local_queue: Receiver<Item>,
    global_queue: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>,
    results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>>,
    errors_collector: Arc<Mutex<Vec<E>>>,
    _marker_context: PhantomData<Context>,
}

impl<Item, Context, Client, Task, T, E> Worker<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    async fn run(self) -> Result<(), PoolError<E>> {
        let mut task_handles = FuturesUnordered::new();

        for task_id in 0..*BATCH_SIZE {
            let worker_clone = Worker {
                id: self.id,
                client: self.client.clone(),
                task_processor: self.task_processor.clone(),
                local_queue: self.local_queue.clone(),
                global_queue: self.global_queue.clone(),
                retry_sender: self.retry_sender.clone(),
                results_collector: self.results_collector.clone(),
                errors_collector: self.errors_collector.clone(),
                _marker_context: PhantomData,
            };
            task_handles.push(tokio::spawn(worker_clone.run_task_processor(task_id)));
        }

        // Use a timeout to prevent hanging indefinitely
        let task_timeout = std::time::Duration::from_secs(60); // 60 second timeout

        while !task_handles.is_empty() {
            match tokio::time::timeout(task_timeout, task_handles.next()).await {
                Ok(Some(result)) => {
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => return Err(e),
                        Err(join_err) => return Err(PoolError::JoinError(join_err)),
                    }
                },
                Ok(None) => {
                    // No more tasks to wait for
                    break;
                },
                Err(_) => {
                    // Timeout occurred
                    warn!("Timeout waiting for worker {} tasks to complete. Some tasks may still be running.", self.id);
                    break;
                }
            }
        }

        debug!("Worker {} completed all tasks or timed out", self.id);
        Ok(())
    }

    async fn run_task_processor(self, task_id: usize) -> Result<(), PoolError<E>> {
        loop {
            let item = tokio::select! {
                biased;
                result = self.local_queue.recv() => {
                    match result {
                        Ok(item) => Some(item),
                        Err(_) => {
                            // Local queue is closed or errored
                            if self.global_queue.is_closed() {
                                // Both channels are now closed
                                trace!(
                                    "Worker {}.{} terminating: Local channel error and Global closed",
                                    self.id,
                                    task_id
                                );
                                None
                            } else {
                                // Continue to try global queue
                                continue;
                            }
                        }
                    }
                },
                result = self.global_queue.recv() => {
                    match result {
                        Ok(item) => Some(item),
                        Err(_) => {
                            // Global queue is closed or errored
                            if self.local_queue.is_closed() {
                                // Both channels are now closed
                                trace!(
                                    "Worker {}.{} terminating: Global channel error and Local closed",
                                    self.id,
                                    task_id
                                );
                                None
                            } else {
                                // Continue to try local queue
                                continue;
                            }
                        }
                    }
                },
            };

            if let Some(item) = item {
                trace!("Worker {}.{} processing item", self.id, task_id);
                match self
                    .task_processor
                    .process(self.id, &self.client, item)
                    .await
                {
                    Ok((item_id, result)) => {
                        self.results_collector.lock().await.push((item_id, result));
                    }
                    Err((error, failed_item)) => {
                        if let Some(retry_tx) = &self.retry_sender {
                            if retry_tx.try_send((error.clone(), failed_item)).is_err() {
                                debug!(
                                    "Retry channel closed or full for worker {}, task {}, collecting error.",
                                    self.id, task_id
                                );
                                self.errors_collector.lock().await.push(error);
                            }
                        } else {
                            self.errors_collector.lock().await.push(error);
                        }
                    }
                }
            } else {
                trace!(
                    "Worker {}.{} terminating: Local closed={}, Global closed={}",
                    self.id,
                    task_id,
                    self.local_queue.is_closed(),
                    self.global_queue.is_closed()
                );
                break;
            }
        }
        Ok(())
    }
}

// Step 5: Modify WorkerPool Struct
pub struct WorkerPool<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    task: Arc<Task>,
    clients: Vec<Arc<Client>>,
    // Channels managed internally by the pool
    worker_txs: Vec<Sender<Item>>,
    worker_rxs: Vec<Receiver<Item>>,
    global_tx: Sender<Item>,
    global_rx: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>,
    retry_rx: Option<Receiver<(E, Item)>>,
    _marker_context: PhantomData<Context>,
    _marker_result: PhantomData<T>,
    _marker_error: PhantomData<E>,
}

// Step 6: Standalone `build` function
#[allow(clippy::too_many_arguments)]
pub async fn build<Item, Context, Task, T, E>(
    config: WorkerPoolConfig<Task>,
    recycle_fn: Option<
        Arc<
            dyn Fn(
                    E,
                    Item,
                )
                    -> futures::future::BoxFuture<'static, Result<Option<Item>, crate::error::Error>>
                + Send
                + Sync,
        >,
    >,
) -> Result<WorkerPool<Item, Context, Object<ClientManager>, Task, T, E>, PoolError<E>>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Object<ClientManager>, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: Debug + Send + Clone + 'static + From<crate::error::Error>,
{
    if config.enable_recycling && recycle_fn.is_none() {
        return Err(PoolError::PoolSetupError(
            "Recycling enabled but no recycle_fn provided".to_string(),
        ));
    }

    let num_workers = *NB_CLIENTS;
    let batch_size = *BATCH_SIZE;

    // --- Channel Creation ---
    let mut worker_txs = Vec::with_capacity(num_workers);
    let mut worker_rxs = Vec::with_capacity(num_workers);
    let worker_bound = config.total_items_hint.saturating_add(1) / num_workers + batch_size;
    for _ in 0..num_workers {
        let (tx, rx) = bounded::<Item>(worker_bound);
        worker_txs.push(tx);
        worker_rxs.push(rx);
    }

    let global_bound = config.total_items_hint + num_workers * batch_size;
    let (global_tx, global_rx) = bounded::<Item>(global_bound);

    let (retry_sender, retry_receiver) = if config.enable_recycling {
        let (tx, rx) = bounded::<(E, Item)>(global_bound);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // --- Client Acquisition ---
    let mut clients = Vec::with_capacity(num_workers);
    for worker_id in 0..num_workers {
        match config
            .network
            .get_client(config.client_config.clone())
            .await
        {
            Ok(client) => clients.push(Arc::new(client)),
            Err(e) => {
                let err_msg = format!("Failed to get client for worker {}: {:?}", worker_id, e);
                error!("{}", err_msg);
                return Err(PoolError::ClientAcquisitionError(err_msg));
            }
        }
    }

    // Create the pool instance
    let pool = WorkerPool {
        task: Arc::new(config.task_processor),
        clients,
        worker_txs,
        worker_rxs,
        global_tx,
        global_rx,
        retry_sender,
        retry_rx: retry_receiver,
        _marker_context: PhantomData,
        _marker_result: PhantomData,
        _marker_error: PhantomData,
    };

    Ok(pool)
}

impl<Item, Context, Task, T, E> WorkerPool<Item, Context, Object<ClientManager>, Task, T, E>
where
    Item: Send + 'static + Debug,
    Context: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Object<ClientManager>, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: Debug + Send + Clone + 'static + From<crate::error::Error>,
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
                        Result<Option<Item>, crate::error::Error>,
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
                debug!("WorkerPool internal recycler task started.");

                let mut recycled_count = 0;
                let mut dropped_count = 0;
                let mut error_count = 0;

                while let Ok((error_cause, item_to_recycle)) = retry_rx.recv().await {
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
                                break;
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

        // Wait for all workers to complete with a timeout
        debug!("Waiting for {} workers to complete...", worker_handles.len());

        // Use a timeout to prevent hanging indefinitely
        let worker_timeout = std::time::Duration::from_secs(10); // 10 second timeout
        let mut completed_workers = 0;

        while !worker_handles.is_empty() {
            match tokio::time::timeout(worker_timeout, worker_handles.next()).await {
                Ok(Some(result)) => {
                    match result {
                        Ok(Ok(())) => {
                            completed_workers += 1;
                            debug!("Worker completed successfully ({}/{})", completed_workers, worker_handles.len() + completed_workers);
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
                Ok(None) => {
                    // No more workers to wait for
                    debug!("No more workers to wait for.");
                    break;
                },
                Err(_) => {
                    // Timeout occurred
                    warn!("Timeout waiting for workers to complete. {} workers completed, {} still running.",
                          completed_workers, worker_handles.len());
                    break;
                }
            }
        }
        debug!("All workers completed or timed out.");

        // Wait for recycler to complete if it exists, with a timeout
        if let Some(handle) = recycler_handle {
            debug!("Waiting for recycler task to complete...");

            // Use a timeout to prevent hanging indefinitely
            let recycler_timeout = std::time::Duration::from_secs(5); // 5 second timeout

            match tokio::time::timeout(recycler_timeout, handle).await {
                Ok(result) => {
                    match result {
                        Ok(Ok(())) => {
                            debug!("Internal recycler task completed successfully.");
                        }
                        Ok(Err(recycler_pool_error)) => {
                            error!("Internal recycler task failed: {:?}", recycler_pool_error);
                        }
                        Err(join_err) => {
                            error!("Internal recycler task panicked: {:?}", join_err);
                            return Err(PoolError::JoinError(join_err));
                        }
                    }
                },
                Err(_) => {
                    // Timeout occurred
                    warn!("Timeout waiting for recycler task to complete. It may still be running.");
                    // We'll continue anyway since we've already closed all channels
                }
            }
        } else {
            debug!("No recycler task to wait for.");
        }

        // Check for errors
        let final_errors = errors_collector.lock().await;
        if !final_errors.is_empty() {
            return Err(PoolError::TaskError(final_errors.first().unwrap().clone()));
        }

        // Return results
        let result = match Arc::try_unwrap(results_collector) {
            Ok(mutex) => {
                let results = mutex.into_inner();
                debug!("WorkerPool completed successfully with {} results.", results.len());
                Ok(results)
            },
            Err(_) => {
                error!("Failed to unwrap results collector Arc");
                Err(PoolError::PoolSetupError(
                    "Failed to unwrap results collector Arc".to_string(),
                ))
            },
        };

        debug!("WorkerPool::run finished.");
        result
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
