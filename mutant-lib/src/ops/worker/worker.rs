use async_channel::{Receiver, Sender};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{debug, trace};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::network::BATCH_SIZE;
use super::async_task::AsyncTask;
use super::error::PoolError;

pub(crate) struct Worker<Item, Context, Client, Task, T, E>
where
    Item: Send + 'static,
    Context: Send + Sync + 'static,
    Client: Send + Sync + 'static,
    Task: AsyncTask<Item, Context, Client, T, E> + Send + Sync + 'static + Clone,
    T: Send + Sync + Clone + 'static,
    E: std::fmt::Debug + Send + Clone + 'static,
{
    pub id: usize,
    pub client: Arc<Client>,
    pub task_processor: Arc<Task>,
    pub local_queue: Receiver<Item>,
    pub global_queue: Receiver<Item>,
    pub retry_sender: Option<Sender<(E, Item)>>,
    pub results_collector: Arc<Mutex<Vec<(Task::ItemId, T)>>>,
    pub errors_collector: Arc<Mutex<Vec<E>>>,
    pub processed_items_counter: Arc<Mutex<usize>>,
    pub active_workers_counter: Arc<Mutex<usize>>,
    pub all_items_processed: Arc<tokio::sync::Notify>,
    pub total_items_hint: usize,
    pub _marker_context: PhantomData<Context>,
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
    pub async fn run(self) -> Result<(), PoolError<E>> {
        let mut task_handles = FuturesUnordered::new();

        // We'll increment the active workers counter once for the worker itself
        // rather than for each task, to avoid overcounting
        {
            let mut counter = self.active_workers_counter.lock().await;
            *counter += 1;
            debug!("Worker {} started. Active workers: {}", self.id, *counter);
        }

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
                processed_items_counter: self.processed_items_counter.clone(),
                active_workers_counter: self.active_workers_counter.clone(),
                all_items_processed: self.all_items_processed.clone(),
                total_items_hint: self.total_items_hint,
                _marker_context: PhantomData,
            };
            task_handles.push(tokio::spawn(worker_clone.run_task_processor(task_id)));
        }

        while !task_handles.is_empty() {
            match task_handles.next().await {
                Some(result) => {
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => return Err(e),
                        Err(join_err) => return Err(PoolError::JoinError(join_err)),
                    }
                },
                None => {
                    // No more tasks to wait for
                    break;
                }
            }
        }

        // We don't need to decrement the active workers counter here anymore
        // It's already decremented in the run_task_processor method when the last task completes
        debug!("Worker {} completed all tasks.", self.id);

        Ok(())
    }

    async fn run_task_processor(self, task_id: usize) -> Result<(), PoolError<E>> {
        loop {
            // Create a properly blocking approach that doesn't consume CPU
            let item = if !self.local_queue.is_closed() && !self.global_queue.is_closed() {
                // Both channels are open, use select to try both
                tokio::select! {
                    biased;
                    result = self.local_queue.recv() => {
                        match result {
                            Ok(item) => Some(item),
                            Err(_) => None, // Local queue closed during receive
                        }
                    },
                    result = self.global_queue.recv() => {
                        match result {
                            Ok(item) => Some(item),
                            Err(_) => None, // Global queue closed during receive
                        }
                    },
                }
            } else if !self.local_queue.is_closed() {
                // Only local queue is open
                trace!(
                    "Worker {}.{}: Only local queue is open, blocking on it",
                    self.id,
                    task_id
                );
                match self.local_queue.recv().await {
                    Ok(item) => Some(item),
                    Err(_) => {
                        trace!(
                            "Worker {}.{} terminating: Local channel closed while waiting",
                            self.id,
                            task_id
                        );
                        None
                    }
                }
            } else if !self.global_queue.is_closed() {
                // Only global queue is open
                trace!(
                    "Worker {}.{}: Only global queue is open, blocking on it",
                    self.id,
                    task_id
                );
                match self.global_queue.recv().await {
                    Ok(item) => Some(item),
                    Err(_) => {
                        trace!(
                            "Worker {}.{} terminating: Global channel closed while waiting",
                            self.id,
                            task_id
                        );
                        None
                    }
                }
            } else {
                // Both channels are closed
                trace!(
                    "Worker {}.{} terminating: Both channels are closed",
                    self.id,
                    task_id
                );
                None
            };

            if let Some(item) = item {
                trace!("Worker {}.{} processing item", self.id, task_id);
                match self
                    .task_processor
                    .process(self.id, &self.client, item)
                    .await
                {
                    Ok((item_id, result)) => {
                        // Increment the processed items counter
                        let mut counter = self.processed_items_counter.lock().await;
                        *counter += 1;
                        let current_count = *counter;
                        drop(counter); // Release the lock

                        // Check if we've processed all items
                        // We notify in these cases:
                        // 1. We've processed exactly the total_items_hint (all expected pads are Confirmed)
                        // 2. We've processed all items that were actually in the queue (might be less than hint)
                        // 3. We've processed some items and both queues are closed
                        if current_count == self.total_items_hint {
                            debug!("Worker {}.{}: Processed {} items (exact match with hint), notifying",
                                   self.id, task_id, current_count);
                            // Notify that all items have been processed
                            self.all_items_processed.notify_waiters();
                        } else if self.local_queue.is_closed() && self.global_queue.is_closed() {
                            // If both queues are closed and we've processed some items,
                            // it means we've processed all actual items (which might be fewer than the hint)
                            debug!("Worker {}.{}: Processed {} items and queues are closed, notifying",
                                   self.id, task_id, current_count);
                            self.all_items_processed.notify_waiters();
                        } else if current_count > 0 {
                            // If we've processed some items, check if this is the last item we expect to process
                            let counter = self.active_workers_counter.lock().await;
                            if *counter <= 1 {
                                debug!("Worker {}.{}: Processed {} items with only {} active workers, notifying",
                                       self.id, task_id, current_count, *counter);
                                drop(counter);
                                self.all_items_processed.notify_waiters();
                            } else {
                                drop(counter);
                            }
                        }

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

                // Check if this is the last task for this worker
                // If all tasks are done, we should decrement the active workers counter
                debug!("Worker {}.{} task completed. No more items to process.", self.id, task_id);

                // Decrement the active workers counter if this is the last task to finish
                // We do this here to ensure the counter is decremented as soon as possible
                // rather than waiting for the entire worker to complete
                if task_id == *BATCH_SIZE - 1 {
                    let mut counter = self.active_workers_counter.lock().await;
                    if *counter > 0 {
                        *counter -= 1;
                    }
                    debug!("Worker {} last task completed. Active workers: {}", self.id, *counter);
                }

                break;
            }
        }
        Ok(())
    }
}
