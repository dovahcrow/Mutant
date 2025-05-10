use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use log::error;
use mutant_protocol::{KeyDetails, StatsResponse, Task, TaskId, TaskListEntry};

// Import our client manager
use crate::app::client_manager;

// Cache expiration time in seconds
const CACHE_EXPIRY_SECONDS: u64 = 5;

// Struct to hold cached data with expiration
struct CachedData<T> {
    data: T,
    timestamp: Instant,
}

impl<T: Clone> CachedData<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            timestamp: Instant::now(),
        }
    }

    fn is_expired(&self) -> bool {
        self.timestamp.elapsed() > Duration::from_secs(CACHE_EXPIRY_SECONDS)
    }

    fn get_data(&self) -> T {
        self.data.clone()
    }
}

// Context struct to manage cached data
pub struct Context {
    // We don't store the client directly since it's not Sync
    connection_state: RwLock<bool>,
    keys_cache: RwLock<Option<CachedData<Vec<KeyDetails>>>>,
    tasks_cache: RwLock<Option<CachedData<Vec<TaskListEntry>>>>,
    stats_cache: RwLock<Option<CachedData<StatsResponse>>>,
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

    // Get list of keys with caching
    pub async fn list_keys(&self) -> (Vec<KeyDetails>, bool) {
        // Check cache first
        {
            let cache = self.keys_cache.read().unwrap();
            if let Some(cached) = &*cache {
                if !cached.is_expired() {
                    return (cached.get_data(), *self.connection_state.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        match client_manager::list_keys().await {
            Ok(keys) => {
                // Update cache and connection state
                *self.keys_cache.write().unwrap() = Some(CachedData::new(keys.clone()));
                *self.connection_state.write().unwrap() = true;
                (keys, true)
            }
            Err(e) => {
                error!("Failed to list keys: {}", e);
                *self.connection_state.write().unwrap() = false;
                (Vec::new(), false)
            }
        }
    }

    // Get list of tasks with caching
    pub async fn list_tasks(&self) -> (Vec<TaskListEntry>, bool) {
        // Check cache first
        {
            let cache = self.tasks_cache.read().unwrap();
            if let Some(cached) = &*cache {
                if !cached.is_expired() {
                    return (cached.get_data(), *self.connection_state.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        match client_manager::list_tasks().await {
            Ok(tasks) => {
                // Update cache and connection state
                *self.tasks_cache.write().unwrap() = Some(CachedData::new(tasks.clone()));
                *self.connection_state.write().unwrap() = true;
                (tasks, true)
            }
            Err(e) => {
                error!("Failed to list tasks: {}", e);
                *self.connection_state.write().unwrap() = false;
                (Vec::new(), false)
            }
        }
    }

    // Get stats with caching
    pub async fn get_stats(&self) -> (Option<StatsResponse>, bool) {
        // Check cache first
        {
            let cache = self.stats_cache.read().unwrap();
            if let Some(cached) = &*cache {
                if !cached.is_expired() {
                    return (Some(cached.get_data()), *self.connection_state.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        match client_manager::get_stats().await {
            Ok(stats) => {
                // Update cache and connection state
                *self.stats_cache.write().unwrap() = Some(CachedData::new(stats.clone()));
                *self.connection_state.write().unwrap() = true;
                (Some(stats), true)
            }
            Err(e) => {
                error!("Failed to get stats: {}", e);
                *self.connection_state.write().unwrap() = false;
                (None, false)
            }
        }
    }

    // Get task details (not cached)
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
        match client_manager::get_task(task_id).await {
            Ok(task) => {
                *self.connection_state.write().unwrap() = true;
                Ok(task)
            }
            Err(e) => {
                error!("Failed to get task details: {}", e);
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        }
    }

    // Stop a task
    pub async fn stop_task(&self, task_id: TaskId) -> Result<(), String> {
        match client_manager::stop_task(task_id).await {
            Ok(_) => {
                // Invalidate tasks cache
                *self.tasks_cache.write().unwrap() = None;
                *self.connection_state.write().unwrap() = true;
                Ok(())
            }
            Err(e) => {
                error!("Failed to stop task: {}", e);
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        }
    }

    // Get a key (not cached)
    pub async fn get_key(&self, name: &str, destination: &str) -> Result<(), String> {
        let res = client_manager::get_key(name, destination).await;

        if res.is_ok() {
            *self.connection_state.write().unwrap() = true;
        } else {
            *self.connection_state.write().unwrap() = false;
        }

        // Refresh the tasks list after a get operation
        let _ = self.list_tasks().await;

        res
    }

    // Invalidate all caches
    pub fn invalidate_caches(&self) {
        *self.keys_cache.write().unwrap() = None;
        *self.tasks_cache.write().unwrap() = None;
        *self.stats_cache.write().unwrap() = None;
    }
}
