use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use log::{error, info};
use mutant_client::MutantClient;
use mutant_protocol::{KeyDetails, StatsResponse, Task, TaskId, TaskListEntry};

// Default WebSocket URL for the daemon
const DEFAULT_WS_URL: &str = "ws://localhost:3030/ws";

// Cache expiration time in seconds
const CACHE_EXPIRY_SECONDS: u64 = 5;

// Struct to hold cached data with expiration
struct CachedData<T> {
    data: T,
    timestamp: Instant,
}

impl<T> CachedData<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            timestamp: Instant::now(),
        }
    }

    fn is_expired(&self) -> bool {
        self.timestamp.elapsed() > Duration::from_secs(CACHE_EXPIRY_SECONDS)
    }
}

// Context struct to manage client and cached data
pub struct Context {
    client: Arc<RwLock<MutantClient>>,
    connected: RwLock<bool>,
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
            client: Arc::new(RwLock::new(MutantClient::new())),
            connected: RwLock::new(false),
            keys_cache: RwLock::new(None),
            tasks_cache: RwLock::new(None),
            stats_cache: RwLock::new(None),
        }
    }

    // Connect to the daemon if not already connected
    pub async fn ensure_connected(&self) -> bool {
        if *self.connected.read().unwrap() {
            return true;
        }

        let mut client = self.client.write().unwrap();
        match client.connect(DEFAULT_WS_URL).await {
            Ok(_) => {
                info!("Connected to daemon");
                *self.connected.write().unwrap() = true;
                true
            }
            Err(e) => {
                error!("Failed to connect to daemon: {:?}", e);
                *self.connected.write().unwrap() = false;
                false
            }
        }
    }

    // Get list of keys with caching
    pub async fn list_keys(&self) -> (Vec<KeyDetails>, bool) {
        // Check cache first
        {
            let cache = self.keys_cache.read().unwrap();
            if let Some(cached) = &*cache {
                if !cached.is_expired() {
                    return (cached.data.clone(), *self.connected.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        if !self.ensure_connected().await {
            return (Vec::new(), false);
        }

        let result = {
            let mut client = self.client.write().unwrap();
            client.list_keys().await
        };

        match result {
            Ok(keys) => {
                // Update cache
                *self.keys_cache.write().unwrap() = Some(CachedData::new(keys.clone()));
                (keys, true)
            }
            Err(e) => {
                error!("Failed to list keys: {:?}", e);
                (Vec::new(), *self.connected.read().unwrap())
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
                    return (cached.data.clone(), *self.connected.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        if !self.ensure_connected().await {
            return (Vec::new(), false);
        }

        let result = {
            let mut client = self.client.write().unwrap();
            client.list_tasks().await
        };

        match result {
            Ok(tasks) => {
                // Update cache
                *self.tasks_cache.write().unwrap() = Some(CachedData::new(tasks.clone()));
                (tasks, true)
            }
            Err(e) => {
                error!("Failed to list tasks: {:?}", e);
                (Vec::new(), *self.connected.read().unwrap())
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
                    return (Some(cached.data.clone()), *self.connected.read().unwrap());
                }
            }
        }

        // Cache expired or not present, fetch fresh data
        if !self.ensure_connected().await {
            return (None, false);
        }

        let result = {
            let mut client = self.client.write().unwrap();
            client.get_stats().await
        };

        match result {
            Ok(stats) => {
                // Update cache
                *self.stats_cache.write().unwrap() = Some(CachedData::new(stats.clone()));
                (Some(stats), true)
            }
            Err(e) => {
                error!("Failed to get stats: {:?}", e);
                (None, *self.connected.read().unwrap())
            }
        }
    }

    // Get task details (not cached)
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
        if !self.ensure_connected().await {
            return Err("Not connected to daemon".to_string());
        }

        let result = {
            let mut client = self.client.write().unwrap();
            client.query_task(task_id).await
        };

        match result {
            Ok(task) => Ok(task),
            Err(e) => {
                error!("Failed to get task details: {:?}", e);
                Err(format!("Failed to get task details: {:?}", e))
            }
        }
    }

    // Stop a task
    pub async fn stop_task(&self, task_id: TaskId) -> Result<(), String> {
        if !self.ensure_connected().await {
            return Err("Not connected to daemon".to_string());
        }

        let result = {
            let mut client = self.client.write().unwrap();
            client.stop_task(task_id).await
        };

        match result {
            Ok(_) => {
                // Invalidate tasks cache
                *self.tasks_cache.write().unwrap() = None;
                Ok(())
            }
            Err(e) => {
                error!("Failed to stop task: {:?}", e);
                Err(format!("Failed to stop task: {:?}", e))
            }
        }
    }

    // Get a key (not cached)
    pub async fn get_key(&self, name: &str, destination: &str) -> Result<(), String> {
        if !self.ensure_connected().await {
            return Err("Not connected to daemon".to_string());
        }

        // Clone the client to avoid borrowing issues with async
        let mut client_clone = {
            let client = self.client.read().unwrap();
            client.clone()
        };

        // Connect the cloned client
        if let Err(e) = client_clone.connect(DEFAULT_WS_URL).await {
            error!("Failed to connect cloned client: {:?}", e);
            return Err(format!("Failed to connect: {:?}", e));
        }

        // Use the cloned client for the get operation
        match client_clone.get(name, destination, false).await {
            Ok((task, _)) => {
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

    // Invalidate all caches
    pub fn invalidate_caches(&self) {
        *self.keys_cache.write().unwrap() = None;
        *self.tasks_cache.write().unwrap() = None;
        *self.stats_cache.write().unwrap() = None;
    }
}
