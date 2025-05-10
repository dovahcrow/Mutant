use std::sync::{Arc, Mutex, RwLock};
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
struct CachedData<T: Clone> {
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
    // We use a Mutex for the client to ensure only one operation uses it at a time
    client: Mutex<Option<MutantClient>>,
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
            client: Mutex::new(None),
            connection_state: RwLock::new(false),
            keys_cache: RwLock::new(None),
            tasks_cache: RwLock::new(None),
            stats_cache: RwLock::new(None),
        }
    }

    // Ensure we have a connected client
    async fn ensure_client(&self) -> Result<(), String> {
        let mut client_guard = match self.client.lock() {
            Ok(guard) => guard,
            Err(e) => return Err(format!("Failed to lock client mutex: {:?}", e)),
        };

        // If we don't have a client or it's not connected, create a new one
        if client_guard.is_none() {
            let mut client = MutantClient::new();
            match client.connect(DEFAULT_WS_URL).await {
                Ok(_) => {
                    info!("Connected to daemon");
                    *client_guard = Some(client);
                    *self.connection_state.write().unwrap() = true;
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to connect to daemon: {:?}", e);
                    *self.connection_state.write().unwrap() = false;
                    Err(format!("Failed to connect: {:?}", e))
                }
            }
        } else {
            Ok(())
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
        if let Err(_) = self.ensure_client().await {
            return (Vec::new(), false);
        }

        let result = {
            let mut client_guard = self.client.lock().unwrap();
            if let Some(client) = &mut *client_guard {
                client.list_keys().await
            } else {
                return (Vec::new(), false);
            }
        };

        match result {
            Ok(keys) => {
                // Update cache
                *self.keys_cache.write().unwrap() = Some(CachedData::new(keys.clone()));
                (keys, true)
            }
            Err(e) => {
                error!("Failed to list keys: {:?}", e);
                (Vec::new(), *self.connection_state.read().unwrap())
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
        if let Err(_) = self.ensure_client().await {
            return (Vec::new(), false);
        }

        let result = {
            let mut client_guard = self.client.lock().unwrap();
            if let Some(client) = &mut *client_guard {
                client.list_tasks().await
            } else {
                return (Vec::new(), false);
            }
        };

        match result {
            Ok(tasks) => {
                // Update cache
                *self.tasks_cache.write().unwrap() = Some(CachedData::new(tasks.clone()));
                (tasks, true)
            }
            Err(e) => {
                error!("Failed to list tasks: {:?}", e);
                (Vec::new(), *self.connection_state.read().unwrap())
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
        if let Err(_) = self.ensure_client().await {
            return (None, false);
        }

        let result = {
            let mut client_guard = self.client.lock().unwrap();
            if let Some(client) = &mut *client_guard {
                client.get_stats().await
            } else {
                return (None, false);
            }
        };

        match result {
            Ok(stats) => {
                // Update cache
                *self.stats_cache.write().unwrap() = Some(CachedData::new(stats.clone()));
                (Some(stats), true)
            }
            Err(e) => {
                error!("Failed to get stats: {:?}", e);
                (None, *self.connection_state.read().unwrap())
            }
        }
    }

    // Get task details (not cached)
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
        if let Err(e) = self.ensure_client().await {
            return Err(e);
        }

        let result = {
            let mut client_guard = self.client.lock().unwrap();
            if let Some(client) = &mut *client_guard {
                client.query_task(task_id).await
            } else {
                return Err("Client not available".to_string());
            }
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
        if let Err(e) = self.ensure_client().await {
            return Err(e);
        }

        let result = {
            let mut client_guard = self.client.lock().unwrap();
            if let Some(client) = &mut *client_guard {
                client.stop_task(task_id).await
            } else {
                return Err("Client not available".to_string());
            }
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
        if let Err(e) = self.ensure_client().await {
            return Err(e);
        }

        // We need to clone the name and destination since they need to live beyond this function
        let name = name.to_string();
        let destination = destination.to_string();

        // Create a new client for this operation to avoid borrowing issues
        let mut new_client = MutantClient::new();
        match new_client.connect(DEFAULT_WS_URL).await {
            Ok(_) => {
                match new_client.get(&name, &destination, false).await {
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
            },
            Err(e) => {
                error!("Failed to connect to daemon: {:?}", e);
                Err(format!("Failed to connect: {:?}", e))
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
