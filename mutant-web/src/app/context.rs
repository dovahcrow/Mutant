use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use log::{error, info};
use mutant_protocol::{KeyDetails, StatsResponse, Task, TaskId, TaskListEntry};

// Default WebSocket URL for the daemon
const DEFAULT_WS_URL: &str = "ws://localhost:3030/ws";

// Cache expiration time in seconds
const CACHE_EXPIRY_SECONDS: u64 = 5;

// Struct to hold cached data with expiration
struct CachedData<T> {
    data: T,
    timestamp: instant::Instant,
}

impl<T: Clone> CachedData<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            timestamp: instant::Instant::now(),
        }
    }

    fn get_data(&self) -> T {
        self.data.clone()
    }
}

// Context struct to manage cached data
pub struct Context {
    // We don't store the client directly since it's not Sync
    connection_state: RwLock<bool>,
    keys_cache: RwLock<Option<Vec<KeyDetails>>>,
    tasks_cache: RwLock<Option<Vec<TaskListEntry>>>,
    stats_cache: RwLock<Option<StatsResponse>>,
}

// Create a global context instance
lazy_static! {
    static ref CONTEXT: Arc<Context> = Arc::new(Context::new());
}

// Public function to get the context
pub fn context() -> Arc<Context> {
    CONTEXT.clone()
}

impl Context {
    fn new() -> Self {
        Self {
            connection_state: RwLock::new(false),
            keys_cache: RwLock::new(None),
            tasks_cache: RwLock::new(None),
            stats_cache: RwLock::new(None),
        }
    }

    // Create a new client and connect to the daemon
    async fn create_client() -> Result<mutant_client::MutantClient, String> {
        let mut client = mutant_client::MutantClient::new();
        
        match client.connect(DEFAULT_WS_URL).await {
            Ok(_) => {
                info!("Connected to daemon");
                Ok(client)
            }
            Err(e) => {
                error!("Failed to connect to daemon: {:?}", e);
                Err(format!("Failed to connect: {:?}", e))
            }
        }
    }

    // Get list of keys with caching
    pub async fn list_keys(&self) -> (Vec<KeyDetails>, bool) {
        let cache = self.keys_cache.read().unwrap();

        if let Some(cached) = &*cache {
            return (cached.clone(), *self.connection_state.read().unwrap());
        }

        return self._list_keys().await;
    }

    async fn _list_keys(&self) -> (Vec<KeyDetails>, bool) {
        match Self::create_client().await {
            Ok(mut client) => {
                match client.list_keys().await {
                    Ok(keys) => {
                        // Update cache and connection state
                        *self.keys_cache.write().unwrap() = Some(keys.clone());
                        *self.connection_state.write().unwrap() = true;
                        (keys, true)
                    }
                    Err(e) => {
                        error!("Failed to list keys: {:?}", e);
                        *self.connection_state.write().unwrap() = true; // Still connected but operation failed
                        (Vec::new(), true)
                    }
                }
            }
            Err(_) => {
                *self.connection_state.write().unwrap() = false;
                (Vec::new(), false)
            }
        }
    }

    // Get list of tasks with caching
    pub async fn list_tasks(&self) -> (Vec<TaskListEntry>, bool) {
        let cache = self.tasks_cache.read().unwrap();

        if let Some(cached) = &*cache {
            return (cached.clone(), *self.connection_state.read().unwrap());
        }

        return self._list_tasks().await;
    }

    async fn _list_tasks(&self) -> (Vec<TaskListEntry>, bool) {
        // Cache expired or not present, fetch fresh data
        match Self::create_client().await {
            Ok(mut client) => {
                match client.list_tasks().await {
                    Ok(tasks) => {
                        // Update cache and connection state
                        *self.tasks_cache.write().unwrap() = Some(tasks.clone());
                        *self.connection_state.write().unwrap() = true;
                        (tasks, true)
                    }
                    Err(e) => {
                        error!("Failed to list tasks: {:?}", e);
                        *self.connection_state.write().unwrap() = true; // Still connected but operation failed
                        (Vec::new(), true)
                    }
                }
            }
            Err(_) => {
                *self.connection_state.write().unwrap() = false;
                (Vec::new(), false)
            }
        }
    }

    // Get stats with caching
    pub async fn get_stats(&self) -> (Option<StatsResponse>, bool) {
        let cache = self.stats_cache.read().unwrap();

        if let Some(cached) = &*cache {
            return (Some(cached.clone()), *self.connection_state.read().unwrap());
        }

        self._get_stats().await
    }

    async fn _get_stats(&self) -> (Option<StatsResponse>, bool) {
        match Self::create_client().await {
            Ok(mut client) => {
                match client.get_stats().await {
                    Ok(stats) => {
                        // Update cache and connection state
                        *self.stats_cache.write().unwrap() = Some(stats.clone());
                        *self.connection_state.write().unwrap() = true;
                        (Some(stats), true)
                    }
                    Err(e) => {
                        error!("Failed to get stats: {:?}", e);
                        *self.connection_state.write().unwrap() = true; // Still connected but operation failed
                        (None, true)
                    }
                }
            }
            Err(_) => {
                *self.connection_state.write().unwrap() = false;
                (None, false)
            }
        }
    }

    // Get task details (not cached)
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
        match Self::create_client().await {
            Ok(mut client) => {
                match client.query_task(task_id).await {
                    Ok(task) => {
                        *self.connection_state.write().unwrap() = true;
                        Ok(task)
                    }
                    Err(e) => {
                        error!("Failed to get task details: {:?}", e);
                        Err(format!("Failed to get task details: {:?}", e))
                    }
                }
            }
            Err(e) => {
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        }
    }

    // Stop a task
    pub async fn stop_task(&self, task_id: TaskId) -> Result<(), String> {
        match Self::create_client().await {
            Ok(mut client) => {
                match client.stop_task(task_id).await {
                    Ok(_) => {
                        // Invalidate tasks cache
                        *self.tasks_cache.write().unwrap() = None;
                        *self.connection_state.write().unwrap() = true;
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to stop task: {:?}", e);
                        Err(format!("Failed to stop task: {:?}", e))
                    }
                }
            }
            Err(e) => {
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        }
    }

    // Get a key (not cached)
    pub async fn get_key(&self, name: &str, destination: &str) -> Result<(), String> {
        let res = match Self::create_client().await {
            Ok(mut client) => {
                match client.get(name, destination, false).await {
                    Ok((task, _)) => {
                        *self.connection_state.write().unwrap() = true;
                        match task.await {
                            Ok(result) => {
                                info!("Get task completed: {:?}", result);
                                Ok(())
                            },
                            Err(e) => {
                                error!("Get task failed: {:?}", e);
                                Err(format!("Get task failed: {:?}", e))
                            }
                        }
                    },
                    Err(e) => {
                        error!("Failed to start get task: {:?}", e);
                        Err(format!("Failed to start get task: {:?}", e))
                    }
                }
            }
            Err(e) => {
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        };

        self._list_tasks().await;

        res
    }

    // Invalidate all caches
    pub fn invalidate_caches(&self) {
        *self.keys_cache.write().unwrap() = None;
        *self.tasks_cache.write().unwrap() = None;
        *self.stats_cache.write().unwrap() = None;
    }
}
