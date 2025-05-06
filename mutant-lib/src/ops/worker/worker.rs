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
