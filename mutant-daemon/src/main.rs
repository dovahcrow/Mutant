use clap::Parser;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use xdg::BaseDirectories;

use mutant_lib::{config::NetworkChoice, MutAnt};
use tokio::sync::RwLock;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use warp::Filter;

mod error;
mod handler;
// mod protocol; // Will be removed later

// Task definitions moved to mutant-protocol crate
use mutant_protocol::{Task, TaskId}; // Import necessary types

// --- Task Management System (Daemon Specific) ---
type TaskMap = Arc<RwLock<HashMap<TaskId, Task>>>;
// --- End Task Management System ---

#[derive(Parser)]
struct Args {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    alphanet: bool,
    #[arg(long)]
    private_key: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Config {
    private_key: String,
    network_choice: NetworkChoice,
}

impl Config {
    fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let xdg_dirs = BaseDirectories::with_prefix("mutant")?;
        let config_path = xdg_dirs.get_config_file("config.json");

        if let Ok(content) = std::fs::read_to_string(&config_path) {
            Ok(serde_json::from_str(&content)?)
        } else {
            Err("No config file found".into())
        }
    }

    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let xdg_dirs = BaseDirectories::with_prefix("mutant")?;
        let config_path = xdg_dirs.get_config_file("config.json");

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    tracing::info!("Starting Mutant Daemon...");

    let network_choice = if args.local {
        NetworkChoice::Devnet
    } else if args.alphanet {
        NetworkChoice::Alphanet
    } else {
        NetworkChoice::Mainnet
    };

    let config = if let Some(private_key) = args.private_key {
        let config = Config {
            private_key,
            network_choice,
        };
        config.save()?;
        config
    } else {
        match Config::load() {
            Ok(config) => config,
            Err(_) => {
                tracing::error!("No private key provided and no config file found");
                return Err("No private key provided and no config file found".into());
            }
        }
    };

    let mutant = Arc::new(match network_choice {
        NetworkChoice::Devnet => {
            tracing::info!("Running in local mode");
            MutAnt::init_local().await
        }
        NetworkChoice::Alphanet => {
            tracing::info!("Running in alphanet mode");
            MutAnt::init_alphanet(&config.private_key).await
        }
        NetworkChoice::Mainnet => {
            tracing::info!("Running in mainnet mode");
            MutAnt::init(&config.private_key).await
        }
    }?);
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
