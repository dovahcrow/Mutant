use async_channel::{Receiver, Sender};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, trace};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

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
    #[allow(dead_code)]
    WorkerError(String), // Error from a worker
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
    completion_notifier: Arc<Notify>,
    total_items: Option<Arc<AtomicUsize>>,
    completed_items_counter: Option<Arc<AtomicUsize>>,
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
                completion_notifier: self.completion_notifier.clone(),
                total_items: self.total_items.clone(),
                completed_items_counter: self.completed_items_counter.clone(),
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
        // Create a channel for signaling completion
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        // Clone all the values we need to move into the monitor task
        let worker_id = self.id;
        let completion_notifier_clone = self.completion_notifier.clone();
        let completed_items_counter_clone = self.completed_items_counter.clone();
        let total_items_clone = self.total_items.clone();
        let local_queue_clone = self.local_queue.clone();
        let global_queue_clone = self.global_queue.clone();

        // Spawn a task to monitor for completion and signal shutdown
        let monitor_task = tokio::spawn(async move {
            loop {
                // Wait for completion notification
                completion_notifier_clone.notified().await;

                // Check if we've completed all work
                if let (Some(counter), Some(total)) =
                    (&completed_items_counter_clone, &total_items_clone)
                {
                    let current_count = counter.load(Ordering::SeqCst);
                    let total_count = total.load(Ordering::SeqCst);
                    if total_count > 0 && current_count >= total_count {
                        // Signal shutdown to the task processor
                        let _ = shutdown_tx.send(()).await;
                        break;
                    }
                }

                // If both queues are closed, signal shutdown
                if local_queue_clone.is_closed() && global_queue_clone.is_closed() {
                    let _ = shutdown_tx.send(()).await;
                    break;
                }

                // Sleep briefly to avoid busy-waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        });

        // Main processing loop
        loop {
            // Try to get work from local queue first, then global queue, or wait for shutdown
            let item = tokio::select! {
                biased;

                result = self.local_queue.recv() => {
                    match result {
                        Ok(item) => {
                            Some(item)
                        },
                        Err(_) => {
                            None
                        },
                    }
                },

                // If local queue is empty, try global queue
                result = self.global_queue.recv() => {
                    match result {
                        Ok(item) => {
                            Some(item)
                        },
                        Err(_) => {
                            None
                        },
                    }
                },

                // Check for shutdown signal
                _ = shutdown_rx.recv() => {
                    None
                }
            };

            if let Some(item) = item {
                // Process the item
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
                        self.results_collector.lock().await.push((item_id, result));

                        if let Some(counter) = &self.completed_items_counter {
                            let previous = counter.fetch_add(1, Ordering::SeqCst);
                            let current = previous + 1;

                            if let Some(total) = &self.total_items {
                                let total_count = total.load(Ordering::SeqCst);
                                if total_count > 0 && current >= total_count {
                                    self.completion_notifier.notify_waiters();
                                }
                            }
                        }
                    }
                    Err((error, failed_item)) => {
                        if let Some(retry_tx) = &self.retry_sender {
                            if retry_tx.send((error.clone(), failed_item)).await.is_err() {
                                self.errors_collector.lock().await.push(error);
                            }
                        } else {
                            self.errors_collector.lock().await.push(error);
                        }
                    }
                }
            } else {
                // No more work and all queues are empty or closed, or we received a shutdown signal
                if self.local_queue.is_closed() && self.global_queue.is_closed() {
                    break;
                }

                // Check if we've completed all work
                if let (Some(counter), Some(total)) =
                    (&self.completed_items_counter, &self.total_items)
                {
                    let current_count = counter.load(Ordering::SeqCst);
                    let total_count = total.load(Ordering::SeqCst);
                    if total_count > 0 && current_count >= total_count {
                        break;
                    }
                }
            }
        }

        // Cancel the monitor task
        monitor_task.abort();

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
    completion_notifier: Arc<Notify>,
    total_items: Option<Arc<AtomicUsize>>,
    completed_items_counter: Option<Arc<AtomicUsize>>,
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
        completion_notifier: Arc<Notify>,
        total_items: Option<Arc<AtomicUsize>>,
        completed_items_counter: Option<Arc<AtomicUsize>>,
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
            completion_notifier,
            total_items,
            completed_items_counter,
            _marker_result: PhantomData,
            _marker_error: PhantomData,
        }
    }

    pub async fn run(mut self) -> Result<Vec<(Task::ItemId, T)>, PoolError<E>> {
        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

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
                retry_sender: self.retry_sender.clone(),
                results_collector: results_collector.clone(),
                errors_collector: errors_collector.clone(),
                completion_notifier: self.completion_notifier.clone(),
                total_items: self.total_items.clone(),
                completed_items_counter: self.completed_items_counter.clone(),
            };

            worker_handles.push(tokio::spawn(async move { worker.run().await }));
        }

        // Wait for all workers to complete
        while let Some(result) = worker_handles.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(join_err) => return Err(PoolError::JoinError(join_err)),
            }
        }

        // Check for errors
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
