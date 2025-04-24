use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use uuid::Uuid;

use mutant_lib::MutAnt;
use tokio::sync::RwLock;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use warp::Filter;

mod error;
mod handler;
mod protocol;

// --- Task Management System ---
type TaskId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    Put,
    Get,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,    // Task created but not yet started
    InProgress, // Task is currently running
    Completed,  // Task finished successfully
    Failed,     // Task failed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProgress {
    // Example: Could include bytes transferred, percentage, etc.
    // For now, keep it simple.
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    // Using Option<String> to hold potential base64 encoded data for Get
    // or error messages.
    data: Option<String>,  // Base64 encoded for Get, None for Put/Error
    error: Option<String>, // Error message if TaskStatus is Failed
}

#[derive(Debug, Clone, Serialize, Deserialize)] // Add Serialize/Deserialize here
pub struct Task {
    id: TaskId,
    task_type: TaskType,
    status: TaskStatus,
    progress: Option<TaskProgress>, // Progress updates
    result: Option<TaskResult>,     // Final result or error
}

impl Task {
    fn new(id: TaskId, task_type: TaskType) -> Self {
        Task {
            id,
            task_type,
            status: TaskStatus::Pending,
            progress: None,
            result: None,
        }
    }
}

type TaskMap = Arc<RwLock<HashMap<TaskId, Task>>>;
// --- End Task Management System ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    tracing::info!("Starting Mutant Daemon...");

    // Initialize MutAnt (using local devnet for now)
    // TODO: Make this configurable
    let mutant = Arc::new(
        MutAnt::init_local()
            .await
            .expect("Failed to initialize MutAnt"),
    );
    tracing::info!("MutAnt initialized successfully.");

    // Initialize Task Management
    let tasks: TaskMap = Arc::new(RwLock::new(HashMap::new()));
    tracing::info!("Task manager initialized.");

    // Define WebSocket route using the actual handler
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::any().map(move || mutant.clone())) // Pass MutAnt instance
        .and(warp::any().map(move || tasks.clone())) // Pass TaskMap
        .map(
            |ws: warp::ws::Ws, mutant_instance: Arc<MutAnt>, task_map: TaskMap| {
                // Use the handler from the handler module
                ws.on_upgrade(move |socket| handler::handle_ws(socket, mutant_instance, task_map))
            },
        );

    // Define server address
    // TODO: Make this configurable
    let addr: SocketAddr = ([127, 0, 0, 1], 3030).into();
    tracing::info!("WebSocket server listening on {}", addr);

    // Start the server
    warp::serve(ws_route).run(addr).await;

    Ok(())
}
