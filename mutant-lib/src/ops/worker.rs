use async_channel::{Receiver, Sender};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, trace};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    // Type to identify the item for ordering results (e.g., chunk index)
    type ItemId: Send + Debug + Clone;

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<Context>,
        client: &Client, // Pass client by reference
        item: Item,
    ) -> Result<(Self::ItemId, TaskResult), (TaskError, Item)>; // Return (ID, Result) on success
}

// Error type for the worker pool itself
#[derive(Debug)]
pub enum PoolError<TaskError>
where
    TaskError: Debug,
{
    TaskError(TaskError),              // Error from within a task's processing
    JoinError(tokio::task::JoinError), // Error joining a task handle
    PoolSetupError(String),            // e.g., failed to get client
    WorkerError(String),               // Error from a worker
}

// Worker structure to manage a set of task processors
// Each worker has EXACTLY ONE client that is shared among its task processors
struct Worker<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    id: usize,
    batch_size: usize,
    context: Arc<Context>,
    client: Arc<Client>, // Each worker has its own client, shared among its task processors
    task_processor: Arc<Task>,
    local_queue: Receiver<Item>,
    global_queue: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>,
    results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>>,
    errors_collector: Arc<Mutex<Vec<E>>>,
}

impl<Item, Context, Client, Task, T, E> Worker<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    async fn run(self) -> Result<(), PoolError<E>> {
        // Create a set of task processors
        let mut task_handles = FuturesUnordered::new();

        // Spawn BATCH_SIZE task processors that share the worker's client
        // This ensures exactly 10 concurrent operations per client
        for task_id in 0..self.batch_size {
            let worker_clone = Worker {
                id: self.id,
                batch_size: self.batch_size,
                context: self.context.clone(),
                client: self.client.clone(), // Share the worker's client among task processors
                task_processor: self.task_processor.clone(),
                local_queue: self.local_queue.clone(),
                global_queue: self.global_queue.clone(),
                retry_sender: self.retry_sender.clone(),
                results_collector: self.results_collector.clone(),
                errors_collector: self.errors_collector.clone(),
            };

            task_handles.push(tokio::spawn(async move {
                worker_clone.run_task_processor(task_id).await
            }));
        }

        // Wait for all task processors to complete
        while let Some(result) = task_handles.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(join_err) => return Err(PoolError::JoinError(join_err)),
            }
        }

        Ok(())
    }

    async fn run_task_processor(self, task_id: usize) -> Result<(), PoolError<E>> {
        let mut local_closed = false;
        let mut global_closed = false;

        // Main processing loop: Continue until BOTH local and global queues are closed.
        loop {
            // Determine if the loop should terminate.
            if local_closed && global_closed {
                trace!(
                    "Worker {}.{} terminating: Both queues closed.",
                    self.id,
                    task_id
                );
                break;
            }

            // Select an item from either local (priority) or global queue,
            // only if the respective queue is not yet closed.
            let item = tokio::select! {
                biased; // Prioritize local queue

                // Attempt to receive from local queue if it's not marked as closed.
                res = self.local_queue.recv(), if !local_closed => {
                    match res {
                        Ok(item) => {
                            trace!("Worker {}.{} received item from local queue", self.id, task_id);
                            Some(item) // Got an item from local queue
                        },
                        Err(_) => { // Local queue is now closed and empty
                            trace!("Worker {}.{} local queue closed", self.id, task_id);
                            local_closed = true; // Mark local queue as closed
                            None // No item received, loop again to check global
                        }
                    }
                },

                // Attempt to receive from global queue if it's not marked as closed.
                res = self.global_queue.recv(), if !global_closed => {
                     match res {
                        Ok(item) => {
                            trace!("Worker {}.{} received item from global queue", self.id, task_id);
                            Some(item) // Got an item from global queue
                        },
                        Err(_) => { // Global queue is now closed and empty
                            trace!("Worker {}.{} global queue closed", self.id, task_id);
                            global_closed = true; // Mark global queue as closed
                            None // No item received, loop again to check local
                        }
                    }
                },

                // This branch is automatically selected if both `if` conditions above are false,
                // meaning both local_closed and global_closed are true.
                else => {
                    // This case should technically be covered by the check at the loop start,
                    // but it acts as a safeguard.
                    trace!("Worker {}.{} both select branches disabled (queues closed), breaking loop", self.id, task_id);
                    break; // Exit loop as both queues are closed
                }
            };

            // If an item was successfully received from either queue, process it.
            if let Some(item) = item {
                trace!("Worker {}.{} processing item", self.id, task_id);
                match self
                    .task_processor
                    .process(
                        self.id,
                        self.context.clone(),
                        &self.client, // Pass client by reference
                        item,
                    )
                    .await
                {
                    Ok((item_id, result)) => {
                        // Successfully processed, collect result.
                        self.results_collector.lock().await.push((item_id, result));
                    }
                    Err((error, failed_item)) => {
                        // Processing failed, attempt to send to retry channel if available.
                        if let Some(retry_tx) = &self.retry_sender {
                            if retry_tx.send((error.clone(), failed_item)).await.is_err() {
                                // Failed to send to retry (channel closed), collect error locally.
                                trace!("Worker {}.{} failed to send item to retry channel (closed?). Collecting error.", self.id, task_id);
                                self.errors_collector.lock().await.push(error);
                            } else {
                                // Successfully sent to retry channel.
                                trace!(
                                    "Worker {}.{} sent item to retry channel.",
                                    self.id,
                                    task_id
                                );
                            }
                        } else {
                            // No retry mechanism, collect the error directly.
                            self.errors_collector.lock().await.push(error);
                        }
                    }
                }
            }
            // If 'item' is None (because a queue was found closed in the select!),
            // the loop continues to the next iteration to check the other queue status
            // or break if both are now closed.
        }

        Ok(())
    }
}

// Define the worker pool struct
// The worker pool manages a set of workers, each with EXACTLY ONE client
// Each worker spawns batch_size task processors that share the worker's client
// This ensures that each client handles exactly batch_size concurrent operations
pub struct WorkerPool<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    num_workers: usize,
    batch_size: usize,
    context: Arc<Context>,
    task: Arc<Task>,
    clients: Vec<Arc<Client>>, // EXACTLY ONE client per worker (critical requirement)
    worker_item_receivers: Vec<Receiver<Item>>,
    global_item_receiver: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>,
    _marker_result: PhantomData<T>,
    _marker_error: PhantomData<E>,
}

impl<Item, Context, Client, Task, T, E> WorkerPool<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_workers: usize,
        batch_size: usize,
        context: Arc<Context>,
        task: Arc<Task>,
        clients: Vec<Arc<Client>>, // EXACTLY ONE client per worker (critical requirement)
        worker_item_receivers: Vec<Receiver<Item>>,
        global_item_receiver: Receiver<Item>,
        retry_sender: Option<Sender<(E, Item)>>,
    ) -> Self {
        assert_eq!(
            worker_item_receivers.len(),
            num_workers,
            "Number of worker item receivers must match num_workers"
        );
        assert_eq!(
            clients.len(),
            num_workers,
            "Number of clients must match num_workers (EXACTLY ONE client per worker)"
        );
        Self {
            num_workers,
            batch_size,
            context,
            task,
            clients,
            worker_item_receivers,
            global_item_receiver,
            retry_sender,
            _marker_result: PhantomData,
            _marker_error: PhantomData,
        }
    }

    pub async fn run(mut self) -> Result<Vec<(Task::ItemId, T)>, PoolError<E>> {
        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

        // Keep a clone of the retry sender for the pool itself.
        // This clone will be dropped *after* all workers have finished,
        // ensuring the recycle_rx doesn't close prematurely.
        let pool_retry_sender = self.retry_sender.clone();

        let worker_rxs = std::mem::take(&mut self.worker_item_receivers);
        let clients = std::mem::take(&mut self.clients);
        let mut worker_handles = FuturesUnordered::new();

        // Create workers with EXACTLY ONE client per worker
        // Each worker will spawn batch_size task processors that share the worker's client
        for ((worker_id, worker_rx), client) in
            worker_rxs.into_iter().enumerate().zip(clients.into_iter())
        {
            let worker = Worker {
                id: worker_id,
                batch_size: self.batch_size,
                context: self.context.clone(),
                client, // Assign exactly ONE client to each worker
                task_processor: self.task.clone(),
                local_queue: worker_rx,
                global_queue: self.global_item_receiver.clone(),
                // Pass the pool's retry_sender; Worker will clone it internally if needed
                retry_sender: self.retry_sender.clone(),
                results_collector: results_collector.clone(),
                errors_collector: errors_collector.clone(),
            };

            worker_handles.push(tokio::spawn(async move { worker.run().await }));
        }

        // Wait for all workers to complete
        while let Some(result) = worker_handles.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    // Worker encountered an irrecoverable error. Drop sender before propagating.
                    drop(pool_retry_sender);
                    return Err(e);
                }
                Err(join_err) => {
                    // Worker panicked. Drop sender before propagating.
                    drop(pool_retry_sender);
                    return Err(PoolError::JoinError(join_err));
                }
            }
        }

        // All workers have finished their execution loops.
        // Now, explicitly drop the pool's clone of the retry sender.
        // This, combined with workers having dropped their clones upon finishing,
        // will cause the recycle_rx channel to close, signaling the recycler task to finish.
        drop(pool_retry_sender);

        // Check for errors that were collected because sending to retry failed or retry was disabled
        let errors = errors_collector.lock().await;
        if !errors.is_empty() {
            return Err(PoolError::TaskError(errors.first().unwrap().clone()));
        }

        // Return results
        match Arc::try_unwrap(results_collector) {
            Ok(mutex) => Ok(mutex.into_inner()),
            Err(_) => Err(PoolError::PoolSetupError(
                "Failed to unwrap results collector Arc".to_string(),
            )),
        }
    }
}
