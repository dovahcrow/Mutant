use async_channel::{Receiver, Sender};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
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
pub struct WorkerPool<Item, Context, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Task: AsyncTask<Item, Context, T, E> + Send + Sync + 'static + Clone,
    T: Send + 'static,
    E: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>,
{
    num_workers: usize,
    batch_size: usize,
    context: Arc<Context>,
    task: Arc<Task>,
    worker_item_receivers: Vec<Receiver<Item>>,
    global_item_receiver: Receiver<Item>,
    retry_sender: Option<Sender<(E, Item)>>, // Use generic E
    completion_notifier: Arc<Notify>,
    total_items: Option<Arc<std::sync::atomic::AtomicUsize>>,
    completed_items_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
    _marker_result: PhantomData<T>,
    _marker_error: PhantomData<E>,
}

impl<Item, Context, Task, T, E> WorkerPool<Item, Context, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static + Clone,
    Task: AsyncTask<Item, Context, T, E> + Send + Sync + 'static + Clone,
    T: Send + 'static,
    E: std::fmt::Debug + Send + Clone + 'static + From<tokio::sync::AcquireError>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_workers: usize,
        batch_size: usize,
        context: Arc<Context>,
        task: Arc<Task>,
        worker_item_receivers: Vec<Receiver<Item>>,
        global_item_receiver: Receiver<Item>,
        retry_sender: Option<Sender<(E, Item)>>, // Use generic E
        completion_notifier: Arc<Notify>,
        total_items: Option<Arc<std::sync::atomic::AtomicUsize>>,
        completed_items_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
    ) -> Self {
        assert_eq!(
            worker_item_receivers.len(),
            num_workers,
            "Number of worker item receivers must match num_workers"
        );
        Self {
            num_workers,
            batch_size,
            context,
            task,
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
        let mut worker_handles: FuturesUnordered<JoinHandle<Result<(), PoolError<E>>>> =
            FuturesUnordered::new();

        let results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let errors_collector: Arc<Mutex<Vec<E>>> = Arc::new(Mutex::new(Vec::new()));

        let worker_rxs = std::mem::take(&mut self.worker_item_receivers);

        for (worker_id, worker_item_rx) in worker_rxs.into_iter().enumerate() {
            let worker_semaphore = Arc::new(Semaphore::new(self.batch_size));
            let worker_context = self.context.clone();
            let worker_global_rx = self.global_item_receiver.clone();
            let worker_retry_tx = self.retry_sender.clone();
            let worker_semaphore_clone = worker_semaphore.clone();
            let task_processor = self.task.clone();
            let worker_completion_notifier = self.completion_notifier.clone();
            let worker_total_items = self.total_items.clone();
            let worker_completed_counter = self.completed_items_counter.clone();
            let worker_results_collector = results_collector.clone();
            let worker_errors_collector = errors_collector.clone();

            worker_handles.push(tokio::spawn(async move {
                let mut processing_tasks = FuturesUnordered::<JoinHandle<()>>::new();
                let mut worker_channel_closed = false;

                loop {
                    if let (Some(counter), Some(total)) =
                        (&worker_completed_counter, &worker_total_items)
                    {
                        let current_count = counter.load(std::sync::atomic::Ordering::SeqCst);
                        let total_count = total.load(std::sync::atomic::Ordering::SeqCst);
                        if total_count > 0 && current_count >= total_count {
                            if worker_item_rx.is_empty() && worker_global_rx.is_empty() {
                                worker_completion_notifier.notify_waiters();
                                break;
                            } else {
                                // debug!("Worker {} detected completion via counter but channels not empty. Processing remaining.", worker_id);
                            }
                        }
                    }

                    let item_option = tokio::select! {
                        biased;

                        _ = worker_completion_notifier.notified() => {
                            if worker_item_rx.is_empty() && worker_global_rx.is_empty() {
                                None
                            } else {
                                match worker_item_rx.try_recv() {
                                    Ok(item) => Some(item),
                                    Err(async_channel::TryRecvError::Empty) => {
                                        match worker_global_rx.try_recv() {
                                            Ok(item) => Some(item),
                                            Err(_) => None
                                        }
                                    },
                                    Err(async_channel::TryRecvError::Closed) => {
                                        worker_channel_closed = true;
                                        match worker_global_rx.try_recv() {
                                            Ok(item) => Some(item),
                                            Err(_) => None
                                        }
                                    }
                                }
                            }
                        },

                        recv_result = worker_item_rx.recv(), if !worker_channel_closed => {
                            match recv_result {
                                Ok(item) => Some(item),
                                Err(_) => {
                                    worker_channel_closed = true;
                                    continue;
                                }
                            }
                        },

                        recv_result = worker_global_rx.recv() => {
                             match recv_result {
                                Ok(item) => Some(item),
                                Err(_) => {
                                    if worker_channel_closed {
                                        None
                                    } else {
                                        continue;
                                    }
                                }
                            }
                        }
                    };

                    match item_option {
                        Some(item) => {
                            let permit = tokio::select! {
                                biased;
                                _ = worker_completion_notifier.notified() => {
                                    continue;
                                },
                                permit_result = worker_semaphore_clone.clone().acquire_owned() => {
                                     match permit_result {
                                         Ok(p) => p,
                                         Err(_) => {
                                            return Err(PoolError::SemaphoreClosed);
                                         }
                                     }
                                }
                            };

                            let task_context = worker_context.clone();
                            let task_retry_tx = worker_retry_tx.clone();
                            let task_item = item;
                            let task_results_collector = worker_results_collector.clone();
                            let task_errors_collector = worker_errors_collector.clone();
                            let task_processor_clone = task_processor.clone();

                            processing_tasks.push(tokio::spawn(async move {
                                let _permit_guard = permit;
                                let task_result: Result<(Task::ItemId, T), (E, Item)> =
                                    task_processor_clone
                                        .process(worker_id, task_context, task_item)
                                        .await;

                                match task_result {
                                    Ok((item_id, result_data)) => {
                                        let mut results = task_results_collector.lock().await;
                                        results.push((item_id, result_data));
                                    }
                                    Err((e, failed_item)) => {
                                        let mut errors = task_errors_collector.lock().await;
                                        let error_clone = e.clone();
                                        errors.push(error_clone);
                                        drop(errors);

                                        if let Some(retry_sender) = task_retry_tx {
                                            if let Err(_send_err) =
                                                retry_sender.send((e, failed_item)).await
                                            {
                                            }
                                        } else {
                                        }
                                    }
                                }
                            }));
                        }
                        None => {
                            break;
                        }
                    }
                }

                while let Some(task_result) = processing_tasks.next().await {
                    if let Err(_e) = task_result {}
                }

                Ok(())
            }));
        }

        while let Some(result) = worker_handles.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(e);
                }
                Err(join_err) => {
                    return Err(PoolError::JoinError(join_err));
                }
            }
        }

        let errors = errors_collector.lock().await;
        if !errors.is_empty() {
            return Err(PoolError::TaskError(errors.first().unwrap().clone()));
        }

        match Arc::try_unwrap(results_collector) {
            Ok(mutex) => Ok(mutex.into_inner()),
            Err(_) => Err(PoolError::PoolSetupError(
                "Failed to unwrap results collector Arc".to_string(),
            )),
        }
    }
}
