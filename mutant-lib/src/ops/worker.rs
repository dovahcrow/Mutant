use crate::network::client::ClientManager;
use crate::network::client::Config as ClientConfig;
use crate::network::Network;
use async_channel::{bounded, Receiver, Sender};
use deadpool::managed::Object;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, error, trace};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

// --- Placeholder Imports ---
// Assume these exist and are properly defined elsewhere
mod placeholder {
    use std::fmt::Debug;
    use std::sync::Arc;
    // Assume NetworkProvider trait is defined (e.g., in crate::network)
    #[async_trait::async_trait]
    pub trait NetworkProvider: Send + Sync + 'static {
        type Client: Send + Sync + 'static;
        type Config: Send + Sync + Clone + 'static; // e.g., crate::network::Config
        type Error: std::error::Error + Send + Sync + 'static;

        async fn get_client(&self, config: Self::Config) -> Result<Self::Client, Self::Error>;
    }

    // Assume Config is defined (e.g., in crate::config)
    pub const NB_CLIENTS: usize = 4; // Example value
    pub const BATCH_SIZE: usize = 10; // Example value

    // Dummy implementation for compilation
    pub struct DummyNetworkProvider;
    #[derive(Clone)]
    pub struct DummyClientConfig;
    pub struct DummyClient;

    #[async_trait::async_trait]
    impl NetworkProvider for DummyNetworkProvider {
        type Client = DummyClient;
        type Config = DummyClientConfig;
        type Error = std::io::Error; // Just an example error type

        async fn get_client(&self, _config: Self::Config) -> Result<Self::Client, Self::Error> {
            Ok(DummyClient)
        }
    }
}
use placeholder::{NetworkProvider, BATCH_SIZE, NB_CLIENTS}; // Use placeholder imports
                                                            // --- End Placeholder Imports ---

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
    WorkerError(String),
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
    batch_size: usize,
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
        let batch_size = BATCH_SIZE;

        for task_id in 0..batch_size {
            let worker_clone = Worker {
                id: self.id,
                batch_size,
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
        loop {
            let item = tokio::select! {
                biased;
                result = self.local_queue.recv() => {
                    match result {
                        Ok(item) => Some(item),
                        Err(_) => None,
                    }
                },
                result = self.global_queue.recv() => {
                    match result {
                        Ok(item) => Some(item),
                        Err(_) => None,
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
    // Receivers for workers
    worker_rxs: Vec<Receiver<Item>>,
    global_rx: Receiver<Item>,
    // Sender for retries (cloned for each worker)
    retry_sender: Option<Sender<(E, Item)>>,

    _marker_context: PhantomData<Context>,
    _marker_result: PhantomData<T>,
    _marker_error: PhantomData<E>,
}

// Handles returned by `build` for the caller to manage
pub struct WorkerPoolHandles<Item, E>
where
    Item: Send + 'static,
    E: Debug + Send + Clone + 'static,
{
    pub worker_txs: Vec<Sender<Item>>,
    pub global_tx: Sender<Item>,
    pub retry_rx: Option<Receiver<(E, Item)>>,
}

// Step 6: Standalone `build` function
#[allow(clippy::too_many_arguments)]
pub async fn build<Item, Context, Task, T, E>(
    config: WorkerPoolConfig<Task>,
) -> Result<
    (
        WorkerPool<Item, Context, Object<ClientManager>, Task, T, E>,
        WorkerPoolHandles<Item, E>,
    ),
    PoolError<E>,
>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Object<ClientManager>, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: Debug + Send + Clone + 'static,
{
    let num_workers = NB_CLIENTS;
    let batch_size = BATCH_SIZE;

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
        worker_rxs,   // Move receivers into the pool
        global_rx,    // Move receiver into the pool
        retry_sender, // Move sender into the pool (will be cloned by workers)
        _marker_context: PhantomData,
        _marker_result: PhantomData,
        _marker_error: PhantomData,
    };

    // Create the handles for the caller
    let handles = WorkerPoolHandles {
        worker_txs,               // Move senders to handles
        global_tx,                // Move sender to handles
        retry_rx: retry_receiver, // Move receiver to handles
    };

    Ok((pool, handles))
}

// Step 7: Implement WorkerPool::run
impl<Item, Context, Task, T, E> WorkerPool<Item, Context, Object<ClientManager>, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Object<ClientManager>, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: Debug + Send + Clone + 'static,
{
    pub async fn run(mut self) -> Result<Vec<(Task::ItemId, T)>, PoolError<E>> {
        let num_workers = NB_CLIENTS;
        let batch_size = BATCH_SIZE;

        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

        // Take ownership of components needed for workers
        let worker_rxs = std::mem::take(&mut self.worker_rxs);
        // Clone Arc fields for workers
        let task = self.task.clone();
        let clients = self.clients.clone(); // Clone the Vec<Arc<Object<ClientManager>>>
                                            // Clone Option<Sender> for workers
        let retry_sender_clone = self.retry_sender.clone();
        // Keep the original retry_sender in `self` to manage its lifetime
        let _pool_retry_sender_lifetime = self.retry_sender;

        let mut worker_handles = FuturesUnordered::new();

        // Spawn workers
        for ((worker_id, worker_rx), client) in
            worker_rxs.into_iter().enumerate().zip(clients.into_iter())
        {
            let worker: Worker<Item, Context, Object<ClientManager>, Task, T, E> = Worker {
                id: worker_id,
                batch_size,
                client, // Arc<Object<ClientManager>> moved
                task_processor: task.clone(),
                local_queue: worker_rx,                   // Receiver is moved
                global_queue: self.global_rx.clone(),     // Clone global receiver for each worker
                retry_sender: retry_sender_clone.clone(), // Clone the Option<Sender>
                results_collector: results_collector.clone(),
                errors_collector: errors_collector.clone(),
                _marker_context: PhantomData::<Context>,
            };
            worker_handles.push(tokio::spawn(worker.run()));
        }

        // Drop the pool's reference to the global receiver *after* workers are spawned
        // Workers hold clones. Dropping this ensures the channel closes eventually
        // when workers + external holders (if any) drop their clones.
        drop(self.global_rx);

        // Await worker completion
        while let Some(result) = worker_handles.next().await {
            match result {
                Ok(Ok(())) => {} // Worker finished ok
                Ok(Err(e)) => {
                    // Worker returned an error (e.g., PoolError::WorkerError)
                    // Propagate the first critical worker error.
                    // Note: TaskErrors are collected via errors_collector normally.
                    error!(
                        "Worker {} failed: {:?}",
                        /* Can't get ID here */ "?", e
                    );
                    // Implicitly drops _pool_retry_sender_lifetime clone here
                    return Err(e);
                }
                Err(join_err) => {
                    // Worker panicked
                    error!("Worker panicked: {:?}", join_err);
                    // Implicitly drops _pool_retry_sender_lifetime clone here
                    return Err(PoolError::JoinError(join_err));
                }
            }
        }

        // All workers have completed without panic or PoolError.
        // The pool's original `retry_sender` clone (`_pool_retry_sender_lifetime`) is dropped now.
        // This allows the retry_rx (held by caller) to close eventually when all worker
        // clones are also dropped.

        // Check for task errors collected via the retry mechanism failure path
        let final_errors = errors_collector.lock().await;
        if !final_errors.is_empty() {
            return Err(PoolError::TaskError(final_errors.first().unwrap().clone()));
        }

        // Return collected results
        match Arc::try_unwrap(results_collector) {
            Ok(mutex) => Ok(mutex.into_inner()),
            Err(_) => Err(PoolError::PoolSetupError(
                "Failed to unwrap results collector Arc".to_string(),
            )),
        }
    }
}

// --- Helper function to check if async channel is closed and empty ---
// (May be useful in schedule_and_run implementation)
// async fn is_channel_fully_drained<ChanItem>(rx: &Receiver<ChanItem>) -> bool {
//     rx.is_closed() && rx.is_empty()
// }

// NOTE: The Worker struct still has placeholder Context=() in its Task bound.
// This needs to be fixed once the Task implementations are updated.
// The generic bounds might need further refinement based on actual Task/Context implementations.
