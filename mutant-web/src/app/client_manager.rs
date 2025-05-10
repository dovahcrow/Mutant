use std::sync::Arc;
use futures::{channel::oneshot, StreamExt};
use lazy_static::lazy_static;
use log::{error, info};
use mutant_client::MutantClient;
use mutant_protocol::{KeyDetails, StatsResponse, Task, TaskId, TaskListEntry};
use wasm_bindgen_futures::spawn_local;

// Default WebSocket URL for the daemon
const DEFAULT_WS_URL: &str = "ws://localhost:3030/ws";

// Commands that can be sent to the client manager
enum ClientCommand {
    ListKeys(oneshot::Sender<Result<Vec<KeyDetails>, String>>),
    ListTasks(oneshot::Sender<Result<Vec<TaskListEntry>, String>>),
    GetStats(oneshot::Sender<Result<StatsResponse, String>>),
    GetTask(TaskId, oneshot::Sender<Result<Task, String>>),
    StopTask(TaskId, oneshot::Sender<Result<(), String>>),
    GetKey(String, String, oneshot::Sender<Result<(), String>>),
    Shutdown,
}

// Singleton channel to the client manager
lazy_static! {
    static ref CLIENT_COMMAND_SENDER: Arc<futures::channel::mpsc::UnboundedSender<ClientCommand>> = {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        spawn_client_manager(rx);
        Arc::new(tx)
    };
}

// Start the client manager in a background task
fn spawn_client_manager(mut rx: futures::channel::mpsc::UnboundedReceiver<ClientCommand>) {
    spawn_local(async move {
        // Create a single client that will be used for all operations
        let mut client = MutantClient::new();
        let mut connected = false;

        // Process commands until shutdown
        while let Some(cmd) = rx.next().await {
            match cmd {
                ClientCommand::ListKeys(sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.list_keys().await {
                        Ok(keys) => {
                            let _ = sender.send(Ok(keys));
                        }
                        Err(e) => {
                            let _ = sender.send(Err(format!("Failed to list keys: {:?}", e)));
                        }
                    }
                }
                ClientCommand::ListTasks(sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.list_tasks().await {
                        Ok(tasks) => {
                            let _ = sender.send(Ok(tasks));
                        }
                        Err(e) => {
                            let _ = sender.send(Err(format!("Failed to list tasks: {:?}", e)));
                        }
                    }
                }
                ClientCommand::GetStats(sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.get_stats().await {
                        Ok(stats) => {
                            let _ = sender.send(Ok(stats));
                        }
                        Err(e) => {
                            let _ = sender.send(Err(format!("Failed to get stats: {:?}", e)));
                        }
                    }
                }
                ClientCommand::GetTask(task_id, sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.query_task(task_id).await {
                        Ok(task) => {
                            let _ = sender.send(Ok(task));
                        }
                        Err(e) => {
                            let _ = sender.send(Err(format!("Failed to get task: {:?}", e)));
                        }
                    }
                }
                ClientCommand::StopTask(task_id, sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.stop_task(task_id).await {
                        Ok(_) => {
                            let _ = sender.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = sender.send(Err(format!("Failed to stop task: {:?}", e)));
                        }
                    }
                }
                ClientCommand::GetKey(name, destination, sender) => {
                    // Ensure we're connected
                    if !connected {
                        match client.connect(DEFAULT_WS_URL).await {
                            Ok(_) => {
                                info!("Connected to daemon");
                                connected = true;
                            }
                            Err(e) => {
                                error!("Failed to connect to daemon: {:?}", e);
                                let _ = sender.send(Err(format!("Failed to connect: {:?}", e)));
                                continue;
                            }
                        }
                    }

                    // Execute the command
                    match client.get(&name, &destination, false).await {
                        Ok((task, _)) => {
                            match task.await {
                                Ok(result) => {
                                    info!("Get task completed: {:?}", result);
                                    let _ = sender.send(Ok(()));
                                },
                                Err(e) => {
                                    error!("Get task failed: {:?}", e);
                                    let _ = sender.send(Err(format!("Get task failed: {:?}", e)));
                                }
                            }
                        },
                        Err(e) => {
                            error!("Failed to start get task: {:?}", e);
                            let _ = sender.send(Err(format!("Failed to start get task: {:?}", e)));
                        }
                    }
                }
                ClientCommand::Shutdown => {
                    break;
                }
            }
        }
    });
}

// Public API to send commands to the client manager
pub async fn list_keys() -> Result<Vec<KeyDetails>, String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::ListKeys(tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}

pub async fn list_tasks() -> Result<Vec<TaskListEntry>, String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::ListTasks(tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}

pub async fn get_stats() -> Result<StatsResponse, String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::GetStats(tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}

pub async fn get_task(task_id: TaskId) -> Result<Task, String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::GetTask(task_id, tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}

pub async fn stop_task(task_id: TaskId) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::StopTask(task_id, tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}

pub async fn get_key(name: &str, destination: &str) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    CLIENT_COMMAND_SENDER.unbounded_send(ClientCommand::GetKey(name.to_string(), destination.to_string(), tx))
        .map_err(|e| format!("Failed to send command: {:?}", e))?;
    rx.await.map_err(|e| format!("Failed to receive response: {:?}", e))?
}
