use async_channel::{Receiver, Sender};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, error, info, warn};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::task::JoinHandle;

// Trait for the actual work function
#[async_trait::async_trait]
pub trait AsyncTask<Item, Context, TaskResult, TaskError>: Send + Sync + 'static
where
    TaskError: Debug + Send,
{
    // Type to identify the item for ordering results (e.g., chunk index)
    type ItemId: Send + Debug + Clone; // Added Clone

    async fn process(
        &self,
        worker_id: usize,
        context: Arc<Context>,
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
    SemaphoreClosed,
}

// Define the worker pool struct
// Removed TaskResult and TaskError from generics as they are not used in the struct fields
pub struct WorkerPool<Item, Context, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Task: AsyncTask<Item, Context, T, E> + Send + Sync + 'static + Clone,
    // TaskResult: Send + 'static,
    // TaskError: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>,
    T: Send + 'static, // Added generic T for AsyncTask's result type
    E: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>, // Added generic E for AsyncTask's error type
{
    num_workers: usize,
    batch_size: usize,
    context: Arc<Context>,
    task: Arc<Task>,
    item_receiver: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>, // Use generic E
    completion_notifier: Arc<Notify>,
    total_items: Option<Arc<std::sync::atomic::AtomicUsize>>,
    completed_items_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
    _marker_result: PhantomData<T>, // Use PhantomData for unused TaskResult (T)
    _marker_error: PhantomData<E>,  // Use PhantomData for unused TaskError (E)
}

impl<Item, Context, Task, T, E> WorkerPool<Item, Context, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Task: AsyncTask<Item, Context, T, E> + Send + Sync + 'static + Clone,
    // TaskResult: Send + 'static,
    T: Send + 'static,
    // TaskError: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>,
    E: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_workers: usize,
        batch_size: usize,
        context: Arc<Context>,
        task: Arc<Task>,
        item_receiver: Receiver<Item>,
        retry_sender: Option<Sender<(E, Item)>>, // Use generic E
        completion_notifier: Arc<Notify>,
        total_items: Option<Arc<std::sync::atomic::AtomicUsize>>,
        completed_items_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
    ) -> Self {
        Self {
            num_workers,
            batch_size,
            context,
            task,
            item_receiver,
            retry_sender,
            completion_notifier,
            total_items,
            completed_items_counter,
            _marker_result: PhantomData, // Initialize PhantomData
            _marker_error: PhantomData,  // Initialize PhantomData
        }
    }

    pub async fn run(self) -> Result<Vec<(Task::ItemId, T)>, PoolError<E>> {
        // Return PoolError<E>
        // Removed global semaphore calculation and creation
        let mut worker_handles: FuturesUnordered<JoinHandle<Result<(), PoolError<E>>>> =
            FuturesUnordered::new();

        // Shared state for collecting results and errors across workers/tasks
        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

        for worker_id in 0..self.num_workers {
            // Create a semaphore specific to this worker
            let worker_semaphore = Arc::new(Semaphore::new(self.batch_size));
            let worker_context = self.context.clone();
            let worker_item_rx = self.item_receiver.clone();
            let worker_retry_tx = self.retry_sender.clone();
            // Clone the worker-specific semaphore for the task
            let worker_semaphore_clone = worker_semaphore.clone();
            let task_processor = self.task.clone(); // Clone Arc for task processor
            let worker_completion_notifier = self.completion_notifier.clone();
            let worker_total_items = self.total_items.clone();
            let worker_completed_counter = self.completed_items_counter.clone();
            let worker_results_collector = results_collector.clone();
            let worker_errors_collector = errors_collector.clone();

            worker_handles.push(tokio::spawn(async move {
                // Collection to store handles of processing tasks spawned by *this* worker
                let mut processing_tasks = FuturesUnordered::<JoinHandle<()>>::new();

                loop {
                    // Check completion based on counter and total expected
                    if let (Some(counter), Some(total)) = (&worker_completed_counter, &worker_total_items) {
                         let current_count = counter.load(std::sync::atomic::Ordering::SeqCst);
                         let total_count = total.load(std::sync::atomic::Ordering::SeqCst);
                         if total_count > 0 && current_count >= total_count { // Ensure total > 0 before comparing
                            // Check if receiver is empty before breaking to process stragglers
                            if worker_item_rx.is_empty() {
                                worker_completion_notifier.notify_waiters();
                                info!("Worker {} detected completion via counter ({} >= {}). Exiting.", worker_id, current_count, total_count);
                                break;
                            } else {
                                debug!("Worker {} detected completion via counter but channel not empty. Processing remaining.", worker_id);
                            }
                         }
                    }

                    let item_option = tokio::select! {
                        biased; // Check notification first
                        _ = worker_completion_notifier.notified() => {
                            // Check if receiver is empty before breaking
                            if worker_item_rx.is_empty() {
                                info!("Worker {} received completion notification. Exiting loop.", worker_id);
                                None::<Item>
                            } else {
                                debug!("Worker {} received notification but channel not empty. Processing remaining.", worker_id);
                                // Try receiving immediately without blocking in select! again
                                match worker_item_rx.try_recv() {
                                     Ok(item) => Some(item),
                                     Err(async_channel::TryRecvError::Empty) => None, // Should break now
                                     Err(async_channel::TryRecvError::Closed) => None, // Channel closed
                                }
                            }
                        },
                        // Then check the item channel
                        recv_result = worker_item_rx.recv() => {
                            match recv_result {
                                Ok(item) => Some(item),
                                Err(_) => {
                                    info!("Worker {} item channel closed. Exiting loop.", worker_id);
                                    None // Channel closed
                                }
                            }
                        }
                    };

                    match item_option {
                        Some(item) => {
                            let permit = tokio::select! {
                                biased;
                                _ = worker_completion_notifier.notified() => {
                                     info!("Worker {} received completion signal while waiting for permit. Exiting loop.", worker_id);
                                    continue; // Go back to check notification / channel
                                },
                                permit_result = worker_semaphore_clone.clone().acquire_owned() => { // Note: cloning semaphore Arc is cheap
                                     match permit_result {
                                         Ok(p) => p,
                                         Err(_) => {
                                             info!("Worker {} semaphore closed. Exiting.", worker_id);
                                             return Err(PoolError::SemaphoreClosed); // Worker task returns error
                                         }
                                     }
                                }
                            };

                             let task_context = worker_context.clone(); // Clone Arc for context
                             // task_processor already cloned above
                             let task_retry_tx = worker_retry_tx.clone();
                             let task_item = item; // Original item moved into task
                             let task_results_collector = worker_results_collector.clone();
                             let task_errors_collector = worker_errors_collector.clone();
                             let task_processor_clone = task_processor.clone(); // Clone Arc for the spawned task

                            // Spawn the processing task and store its handle
                            processing_tasks.push(tokio::spawn(async move {
                                let _permit_guard = permit; // Ensure permit is released when task finishes
                                debug!("Task (Worker {}) started processing.", worker_id);

                                let task_result: Result<(Task::ItemId, T), (E, Item)> = task_processor_clone.process(worker_id, task_context, task_item).await;

                                match task_result {
                                    Ok((item_id, result_data)) => {
                                         debug!("Task (Worker {}) finished successfully for {:?}.", worker_id, item_id);
                                         let mut results = task_results_collector.lock().await;
                                         results.push((item_id, result_data));
                                    }
                                    Err((e, failed_item)) => {
                                         error!("Task (Worker {}) failed: {:?}. Adding to errors and attempting retry/recycle.", worker_id, e);
                                         // Lock errors vector first
                                         let mut errors = task_errors_collector.lock().await;
                                         // Clone the error before pushing, as it might be moved to retry_sender
                                         let error_clone = e.clone();
                                         errors.push(error_clone);
                                         // Drop the lock before potentially blocking await on send
                                         drop(errors);

                                         if let Some(retry_sender) = task_retry_tx {
                                             // Send the error and the item that failed back to the retry channel
                                             if let Err(send_err) = retry_sender.send((e, failed_item)).await {
                                                 error!("Worker {} failed to send item back to retry queue: {}", worker_id, send_err);
                                                 // If sending fails, it's bad. Maybe signal a PoolError?
                                             }
                                         } else {
                                             warn!("Task (Worker {}) failed but no retry channel configured.", worker_id);
                                        }
                                    }
                                }
                                debug!("Task (Worker {}) finished, permit released.", worker_id);
                            }));
                            // We don't collect join handles here anymore, just let them run detached.
                            // Pool completion depends on workers finishing their loops.
                        }
                        None => {
                             info!("Worker {} received None from item channel select. Exiting loop.", worker_id);
                            break; // Channel closed or notified and empty
                        }
                    }
                }

                // Worker task finished its loop, now wait for its spawned processing tasks
                info!(
                    "Worker {} main loop finished. Waiting for {} outstanding processing tasks.",
                    worker_id,
                    processing_tasks.len()
                );
                while let Some(task_result) = processing_tasks.next().await {
                    if let Err(e) = task_result {
                        // Log join errors from individual processing tasks
                        // The main pool error handling might catch logical errors later
                        error!(
                            "Worker {} processing task panicked or failed to join: {:?}",
                            worker_id, e
                        );
                        // Consider if this should propagate an error up to the worker_handle
                        // For now, just logging. If a task panicked, results might be incomplete.
                    }
                }
                info!(
                    "Worker {} finished waiting for processing tasks.",
                    worker_id
                );

                Ok(()) // Indicate worker finished ok (including waiting for its tasks)
            }));
        }

        info!("Waiting for {} worker loops to complete.", self.num_workers);

        while let Some(result) = worker_handles.next().await {
            match result {
                Ok(Ok(())) => { /* Worker loop finished cleanly */ }
                Ok(Err(e)) => {
                    // Worker loop returned an error (e.g., semaphore closed)
                    error!("Worker loop finished with error: {:?}", e);
                    // Propagate the pool error
                    return Err(e);
                }
                Err(join_err) => {
                    error!(
                        "Worker main loop panicked or failed to join: {:?}",
                        join_err
                    );
                    return Err(PoolError::JoinError(join_err)); // Propagate join error
                }
            }
        }

        info!(
            "All {} worker loops finished. Checking collected errors.",
            self.num_workers
        );

        // Check collected errors
        let errors = errors_collector.lock().await;
        if !errors.is_empty() {
            error!("{} errors occurred during processing.", errors.len());
            // Return the first error encountered, wrapped in PoolError::TaskError
            // `.clone()` is okay because TaskError has Clone constraint
            return Err(PoolError::TaskError(errors.first().unwrap().clone()));
        }

        // Return collected results by unwrapping Arc and Mutex
        match Arc::try_unwrap(results_collector) {
            Ok(mutex) => Ok(mutex.into_inner()),
            Err(_) => Err(PoolError::PoolSetupError(
                "Failed to unwrap results collector Arc".to_string(),
            )),
        }
    }
}
