use std::sync::{Arc, RwLock};

use lazy_static::lazy_static;
use log::{error, info};
use mutant_protocol::{KeyDetails, StatsResponse, Task, TaskId, TaskListEntry};

// Import our client manager
use crate::app::client_manager;

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

    // Get list of keys with caching
    pub async fn list_keys(&self) -> (Vec<KeyDetails>, bool) {
        // Check cache first
        {
            let cache = self.keys_cache.read().unwrap();
            let conn_state = self.connection_state.read().unwrap();

            if let Some(cached) = &*cache {
                return (cached.clone(), *conn_state);
            }
        }

        // No cache, fetch fresh data
        info!("No cached keys, fetching from daemon");

        // Create a simple direct call to avoid complex operations
        let result = client_manager::list_keys().await;

        if let Ok(keys) = result {
            info!("Successfully retrieved {} keys from daemon", keys.len());

            // Create a minimal copy to avoid complex operations
            let mut safe_keys = Vec::with_capacity(keys.len());

            for k in keys {
                let key_detail = KeyDetails {
                    key: k.key,
                    total_size: k.total_size,
                    pad_count: k.pad_count,
                    confirmed_pads: k.confirmed_pads,
                    is_public: k.is_public,
                    public_address: k.public_address,
                };

                safe_keys.push(key_detail);
            }

            // Update cache and connection state
            {
                let mut cache = self.keys_cache.write().unwrap();
                let mut conn_state = self.connection_state.write().unwrap();

                *cache = Some(safe_keys.clone());
                *conn_state = true;
            }

            (safe_keys, true)
        } else {
            error!("Failed to list keys: {:?}", result.err());

            // Update connection state
            {
                let mut conn_state = self.connection_state.write().unwrap();
                *conn_state = false;
            }

            (Vec::new(), false)
        }
    }

    // Get list of tasks with caching
    pub async fn list_tasks(&self) -> (Vec<TaskListEntry>, bool) {
        // Check cache first
        {
            let cache = self.tasks_cache.read().unwrap();
            let conn_state = self.connection_state.read().unwrap();

            if let Some(cached) = &*cache {
                return (cached.clone(), *conn_state);
            }
        }

        // No cache, fetch fresh data
        info!("No cached tasks, fetching from daemon");

        // Create a simple direct call to avoid complex operations
        let result = client_manager::list_tasks().await;

        if let Ok(tasks) = result {
            info!("Successfully retrieved {} tasks from daemon", tasks.len());

            // Create a minimal copy to avoid complex operations
            let mut safe_tasks = Vec::with_capacity(tasks.len());

            for t in tasks {
                let task_entry = TaskListEntry {
                    task_id: t.task_id,
                    status: t.status,
                    task_type: t.task_type,
                };

                safe_tasks.push(task_entry);
            }

            // Update cache and connection state
            {
                let mut cache = self.tasks_cache.write().unwrap();
                let mut conn_state = self.connection_state.write().unwrap();

                *cache = Some(safe_tasks.clone());
                *conn_state = true;
            }

            (safe_tasks, true)
        } else {
            error!("Failed to list tasks: {:?}", result.err());

            // Update connection state
            {
                let mut conn_state = self.connection_state.write().unwrap();
                *conn_state = false;
            }

            (Vec::new(), false)
        }
    }

    // Get stats with caching
    pub async fn get_stats(&self) -> (Option<StatsResponse>, bool) {
        // Check cache first
        {
            let cache = self.stats_cache.read().unwrap();
            let conn_state = self.connection_state.read().unwrap();

            if let Some(cached) = &*cache {
                return (Some(cached.clone()), *conn_state);
            }
        }

        // No cache, fetch fresh data
        info!("No cached stats, fetching from daemon");

        // Create a simple direct call to avoid complex operations
        let result = client_manager::get_stats().await;

        if let Ok(stats) = result {
            info!("Successfully retrieved stats from daemon");

            // Create a safe copy of the stats
            let safe_stats = StatsResponse {
                total_keys: stats.total_keys,
                total_pads: stats.total_pads,
                occupied_pads: stats.occupied_pads,
                free_pads: stats.free_pads,
                pending_verify_pads: stats.pending_verify_pads,
            };

            // Update cache and connection state
            {
                let mut cache = self.stats_cache.write().unwrap();
                let mut conn_state = self.connection_state.write().unwrap();

                *cache = Some(safe_stats.clone());
                *conn_state = true;
            }

            (Some(safe_stats), true)
        } else {
            error!("Failed to get stats: {:?}", result.err());

            // Update connection state
            {
                let mut conn_state = self.connection_state.write().unwrap();
                *conn_state = false;
            }

            (None, false)
        }
    }

    // Get task details (not cached)
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
        // Safely call the client manager
        let result = client_manager::get_task(task_id).await;

        match result {
            Ok(task) => {
                // Create a safe copy of the task
                let safe_task = Task {
                    id: task.id,
                    task_type: task.task_type,
                    status: task.status,
                    progress: task.progress.clone(),
                    result: task.result.clone(),
                    key: task.key.clone(),
                };

                // Update connection state
                *self.connection_state.write().unwrap() = true;

                Ok(safe_task)
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
        // Safely call the client manager
        let result = client_manager::stop_task(task_id).await;

        match result {
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
        // Safely call the client manager
        let result = client_manager::get_key(name, destination).await;

        let res = match result {
            Ok(_) => {
                *self.connection_state.write().unwrap() = true;
                Ok(())
            },
            Err(e) => {
                error!("Failed to get key: {}", e);
                *self.connection_state.write().unwrap() = false;
                Err(e)
            }
        };

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

    // Force a reconnection to the daemon
    pub async fn reconnect(&self) -> Result<(), String> {
        // Invalidate all caches
        self.invalidate_caches();

        // Set connection state to false
        *self.connection_state.write().unwrap() = false;

        // Safely call the client manager
        let result = client_manager::reconnect().await;

        match result {
            Ok(_) => {
                info!("Successfully reconnected to daemon");
                Ok(())
            },
            Err(e) => {
                error!("Failed to reconnect to daemon: {}", e);
                Err(e)
            }
        }
    }
}
