use std::{collections::{BTreeMap, HashMap}, sync::{Arc, Mutex, RwLock}};

use lazy_static::lazy_static;
use log::{error, info};
use mutant_protocol::{KeyDetails, StatsResponse, StorageMode, TaskListEntry};

// Import our client manager
use crate::{Client, ClientSender};

pub struct ProgressOperation {
    pub nb_to_reserve: usize,
    pub nb_reserved: usize,
    pub total_pads: usize,
    pub nb_written: usize,
    pub nb_confirmed: usize,
}

pub struct Progress {
    pub operation: BTreeMap<String, ProgressOperation>,
}

// Context struct to manage cached data
pub struct Context {
    client: ClientSender,
    keys_cache: Arc<RwLock<Vec<KeyDetails>>>,
    tasks_cache: Arc<RwLock<Vec<TaskListEntry>>>,
    stats_cache: Arc<RwLock<Option<StatsResponse>>>,
    put_progress: Arc<RwLock<HashMap<String, Arc<RwLock<Progress>>>>>,
    get_progress: Arc<RwLock<HashMap<String, Arc<RwLock<Progress>>>>>,
}

// Create a global context instance
lazy_static! {
    static ref CONTEXT: Arc<Mutex<Option<Arc<Context>>>> = Arc::new(Mutex::new(None));
}

pub async fn init_context() {
    let context = Context::new().await;
    *CONTEXT.lock().unwrap() = Some(Arc::new(context));
}

// Public function to get the context
pub fn context() -> Arc<Context> {
    CONTEXT.lock().unwrap().as_ref().unwrap().clone()
}

impl Context {
    async fn new() -> Self {
        let client = Client::spawn().await;

        Self {
            client,
            keys_cache: Arc::new(RwLock::new(Vec::new())),
            tasks_cache: Arc::new(RwLock::new(Vec::new())),
            stats_cache: Arc::new(RwLock::new(None)),
            put_progress: Arc::new(RwLock::new(HashMap::new())),
            get_progress: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn list_keys(&self) -> Vec<KeyDetails> {
        info!("Fetching keys from daemon");

        // Create a simple direct call to avoid complex operations
        let result = self.client.list_keys().await;

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

            // Update cache
            {
                let mut cache = self.keys_cache.write().unwrap();

                *cache = safe_keys.clone();
            }

            info!("Updated keys cache with {} keys", safe_keys.len());

            safe_keys
        } else {
            error!("Failed to list keys: {:?}", result.err());

            Vec::new()
        }
    }

    // Get list of tasks from cache only
    pub async fn list_tasks(&self) -> Vec<TaskListEntry> {
        info!("Fetching tasks from daemon");

        // Create a simple direct call to avoid complex operations
        let result = self.client.list_tasks().await;

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

                *cache = safe_tasks.clone();
            }

            safe_tasks
        } else {
            error!("Failed to list tasks: {:?}", result.err());

            Vec::new()
        }
    }

    // Get stats from daemon and update cache
    pub async fn get_stats(&self) -> Option<StatsResponse> {
        info!("Fetching stats from daemon");

        // Create a simple direct call to avoid complex operations
        let result = self.client.get_stats().await;

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
                *cache = Some(safe_stats.clone());
            }

            Some(safe_stats)
        } else {
            error!("Failed to get stats: {:?}", result.err());
            None
        }
    }

    // // Get task details (not cached)
    // pub async fn get_task(&self, task_id: TaskId) -> Result<Task, String> {
    //     self._get_task(task_id).await
    // }

    // // Get task details directly from daemon
    // pub async fn _get_task(&self, task_id: TaskId) -> Result<Task, String> {
    //     info!("Fetching task details for task {} from daemon", task_id);

    //     // Safely call the client manager
    //     let result = client_manager::get_task(task_id).await;

    //     match result {
    //         Ok(task) => {
    //             // Create a safe copy of the task
    //             let safe_task = Task {
    //                 id: task.id,
    //                 task_type: task.task_type,
    //                 status: task.status,
    //                 progress: task.progress.clone(),
    //                 result: task.result.clone(),
    //                 key: task.key.clone(),
    //             };

    //             Ok(safe_task)
    //         }
    //         Err(e) => {
    //             error!("Failed to get task details: {}", e);
    //             Err(e)
    //         }
    //     }
    // }

    // // Stop a task
    // pub async fn stop_task(&self, task_id: TaskId) -> Result<(), String> {
    //     self._stop_task(task_id).await
    // }

    // // Stop a task directly from daemon
    // pub async fn _stop_task(&self, task_id: TaskId) -> Result<(), String> {
    //     info!("Stopping task {} via daemon", task_id);

    //     // Safely call the client manager
    //     let result = client_manager::stop_task(task_id).await;

    //     match result {
    //         Ok(_) => {
    //             // Invalidate tasks cache
    //             *self.tasks_cache.write().unwrap() = None;
    //             Ok(())
    //         }
    //         Err(e) => {
    //             error!("Failed to stop task: {}", e);
    //             Err(e)
    //         }
    //     }
    // }

    // Get a key (not cached)
    pub async fn get_key(&self, name: &str, destination: &str) -> Result<(), String> {
        info!("Getting key {} via daemon", name);

        // Safely call the client manager
        let result = self.client.get(name.to_string(), Some(destination.to_string()), false).await;

        match result {
            Ok(_) => {
                Ok(())
            },
            Err(e) => {
                error!("Failed to get key: {}", e);
                Err(e)
            }
        }
    }

    // Get file content directly without saving to disk
    pub async fn get_file_content(&self, name: &str, is_public: bool) -> Result<String, String> {
        info!("Getting file content for key {} via daemon (is_public={})", name, is_public);

        // Call the client with no destination to stream the data
        info!("Calling client.get with streaming enabled");
        let result = self.client.get(name.to_string(), None, is_public).await;

        match result {
            Ok((task_result, Some(data))) => {
                info!("Successfully retrieved file content for key {}, size: {} bytes", name, data.len());
                info!("Task result: {:?}", task_result);

                // Try to convert the data to a string
                match String::from_utf8(data) {
                    Ok(content) => {
                        info!("Successfully converted data to UTF-8 string, length: {}", content.len());
                        Ok(content)
                    },
                    Err(_) => {
                        // If it's not valid UTF-8, return a binary data message
                        error!("Data is not valid UTF-8, cannot display as text");
                        Err("File contains binary data that cannot be displayed as text".to_string())
                    }
                }
            },
            Ok((task_result, None)) => {
                error!("No data received for key {}, task result: {:?}", name, task_result);
                Err("No data received".to_string())
            },
            Err(e) => {
                error!("Failed to get file content: {}", e);
                Err(e)
            }
        }
    }

    // Get file binary data directly without saving to disk
    pub async fn get_file_binary(&self, name: &str, is_public: bool) -> Result<Vec<u8>, String> {
        info!("Getting binary file content for key {} via daemon (is_public={})", name, is_public);

        // Create a progress object for tracking this get operation
        let (get_id, progress) = self.create_get_progress(name);
        info!("Created progress tracking with ID: {}", get_id);

        // Call the client with no destination to stream the data
        info!("Calling client.get with streaming enabled");
        let result = self.client.get(name.to_string(), None, is_public).await;

        match result {
            Ok((task_result, Some(data))) => {
                info!("Successfully retrieved binary file content for key {}, size: {} bytes", name, data.len());
                info!("Task result: {:?}", task_result);

                // Mark the operation as complete in the progress object
                {
                    let mut progress_guard = progress.write().unwrap();
                    if let Some(op) = progress_guard.operation.get_mut("get") {
                        op.nb_confirmed = op.total_pads;
                        info!("Marked get operation as complete in progress object");
                    }
                }

                Ok(data)
            },
            Ok((task_result, None)) => {
                error!("No data received for key {}, task result: {:?}", name, task_result);
                Err("No data received".to_string())
            },
            Err(e) => {
                error!("Failed to get file content: {}", e);
                Err(e)
            }
        }
    }

    pub fn get_key_cache(&self) -> Arc<RwLock<Vec<KeyDetails>>> {
        self.keys_cache.clone()
    }

    pub fn get_task_cache(&self) -> Arc<RwLock<Vec<TaskListEntry>>> {
        self.tasks_cache.clone()
    }

    pub fn get_stats_cache(&self) -> Arc<RwLock<Option<StatsResponse>>> {
        self.stats_cache.clone()
    }

    // Create a new progress object for tracking a put operation
    pub fn create_progress(&self, key: &str, filename: &str) -> (String, Arc<RwLock<Progress>>) {
        // Generate a unique ID for this put operation
        let put_id = format!("put_{}_{}", key, filename);

        // Create a new Progress object
        let progress = Arc::new(RwLock::new(Progress {
            operation: BTreeMap::new(),
        }));

        // Store the progress in our map
        {
            let mut put_progress = self.put_progress.write().unwrap();
            put_progress.insert(put_id.clone(), progress.clone());
        }

        info!("Created progress object with ID: {}", put_id);
        (put_id, progress)
    }

    // Put a key (not cached)
    pub async fn put(
        &self,
        key: &str,
        data: Vec<u8>,
        filename: &str,
        mode: StorageMode,
        public: bool,
        no_verify: bool,
        progress_opt: Option<(String, Arc<RwLock<Progress>>)>,
    ) -> Result<(String, Arc<RwLock<Progress>>), String> {
        info!("Putting key {} via daemon", key);

        // Use the provided progress object or create a new one
        let (put_id, progress) = match progress_opt {
            Some((id, prog)) => (id, prog),
            None => self.create_progress(key, filename)
        };

        // Safely call the client manager with the progress object
        let result = self.client.put(
            key.to_string(),
            data,
            filename.to_string(),
            mode,
            public,
            no_verify,
            Some(progress.clone())
        ).await;

        match result {
            Ok(_task_result) => {
                // Return the put_id and progress
                Ok((put_id, progress))
            },
            Err(e) => {
                error!("Failed to put key: {}", e);
                Err(e)
            }
        }
    }

    // Create a new progress object for tracking a get operation
    pub fn create_get_progress(&self, key: &str) -> (String, Arc<RwLock<Progress>>) {
        // Generate a unique ID for this get operation
        let get_id = format!("get_{}", key);

        // Create a new Progress object
        let progress = Arc::new(RwLock::new(Progress {
            operation: BTreeMap::new(),
        }));

        // Store the progress in our map
        {
            let mut get_progress = self.get_progress.write().unwrap();
            get_progress.insert(get_id.clone(), progress.clone());
        }

        info!("Created get progress object with ID: {}", get_id);
        (get_id, progress)
    }

    // Get a progress object for a put operation
    pub fn get_put_progress(&self, put_id: &str) -> Option<Arc<RwLock<Progress>>> {
        let put_progress = self.put_progress.read().unwrap();
        put_progress.get(put_id).cloned()
    }

    // Get a progress object for a get operation
    pub fn get_get_progress(&self, get_id: &str) -> Option<Arc<RwLock<Progress>>> {
        let get_progress = self.get_progress.read().unwrap();
        get_progress.get(get_id).cloned()
    }

    // Get the client sender
    pub fn get_client_sender(&self) -> Arc<ClientSender> {
        Arc::new(self.client.clone())
    }

    pub async fn mv(&self, old_key: &str, new_key: &str) -> Result<(), String> {
        info!("Renaming key '{}' to '{}' via daemon", old_key, new_key);

        match self.client.mv(old_key.to_string(), new_key.to_string()).await {
            Ok(_) => {
                info!("Successfully renamed key '{}' to '{}'", old_key, new_key);
                // Refresh the key list
                let _ = self.list_keys().await;
                Ok(())
            },
            Err(e) => {
                error!("Failed to rename key '{}' to '{}': {}", old_key, new_key, e);
                Err(e)
            }
        }
    }
}
