use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use xdg::BaseDirectories;

use mutant_lib::{config::NetworkChoice, MutAnt};
use tokio::sync::RwLock;
use warp::Filter;

use crate::error::Error;
use crate::handlers;
use crate::handlers::TaskMap;
use crate::wallet;

use tokio::signal;

#[derive(serde::Deserialize, serde::Serialize)]
struct NetworkConfig {
    private_key: String,
}

#[derive(serde::Deserialize, serde::Serialize, Default)]
pub struct Config {
    mainnet: Option<NetworkConfig>,
    devnet: Option<NetworkConfig>,
    alphanet: Option<NetworkConfig>,
}

impl Config {
    pub fn load() -> Result<Self, Error> {
        let xdg_dirs = BaseDirectories::with_prefix("mutant")?;
        let config_path = xdg_dirs.get_config_file("config.json");

        if let Ok(content) = std::fs::read_to_string(&config_path) {
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<(), Error> {
        let xdg_dirs = BaseDirectories::with_prefix("mutant")?;
        let config_path = xdg_dirs.get_config_file("config.json");

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    pub fn get_private_key(&self, network_choice: NetworkChoice) -> Option<&str> {
        match network_choice {
            NetworkChoice::Mainnet => self.mainnet.as_ref().map(|c| c.private_key.as_str()),
            NetworkChoice::Devnet => self.devnet.as_ref().map(|c| c.private_key.as_str()),
            NetworkChoice::Alphanet => self.alphanet.as_ref().map(|c| c.private_key.as_str()),
        }
    }

    pub fn set_private_key(&mut self, network_choice: NetworkChoice, private_key: String) {
        let config = NetworkConfig { private_key };
        match network_choice {
            NetworkChoice::Mainnet => self.mainnet = Some(config),
            NetworkChoice::Devnet => self.devnet = Some(config),
            NetworkChoice::Alphanet => self.alphanet = Some(config),
        }
    }
}

pub struct AppOptions {
    pub local: bool,
    pub alphanet: bool,
    pub private_key: Option<String>,
}

pub async fn run(options: AppOptions) -> Result<(), Error> {
    // check if the daemon is running by checking the /tmp/mutant-daemon.lock file
    if std::fs::File::open("/tmp/mutant-daemon.lock").is_ok() {
        println!("Mutant Daemon is already running");
        return Ok(());
    }

    // Create the lock file. It will be held until the process exits.
    let _lock_file =
        lockfile::Lockfile::create("/tmp/mutant-daemon.lock").map_err(Error::Lockfile)?;

    // put the pid of the daemon in the lock file
    // Note: The lockfile crate typically manages this implicitly through the file lock.
    // Writing the PID might be redundant or could be handled differently.
    std::fs::write("/tmp/mutant-daemon.lock", std::process::id().to_string())?;

    tracing::info!("Starting Mutant Daemon...");

    let network_choice = if options.local {
        NetworkChoice::Devnet
    } else if options.alphanet {
        NetworkChoice::Alphanet
    } else {
        NetworkChoice::Mainnet
    };

    let mut config = Config::load()?;

    let private_key = if let Some(private_key) = options.private_key {
        config.set_private_key(network_choice, private_key.clone());
        config.save()?;
        private_key
    } else {
        match config.get_private_key(network_choice) {
            Some(key) => key.to_string(),
            None => {
                let private_key = wallet::scan_and_select_wallet().await?;
                config.set_private_key(network_choice, private_key.clone());
                config.save()?;
                private_key
            }
        }
    };

    let mutant = Arc::new(
        match network_choice {
            NetworkChoice::Devnet => {
                tracing::info!("Running in local mode");
                MutAnt::init_local().await
            }
            NetworkChoice::Alphanet => {
                tracing::info!("Running in alphanet mode");
                MutAnt::init_alphanet(&private_key).await
            }
            NetworkChoice::Mainnet => {
                tracing::info!("Running in mainnet mode");
                MutAnt::init(&private_key).await
            }
        }
        .map_err(Error::MutAnt)?,
    );
    tracing::info!("MutAnt initialized successfully.");

    // Initialize Task Management
    let tasks: TaskMap = Arc::new(RwLock::new(HashMap::new()));
    tracing::info!("Task manager initialized.");

    // Define WebSocket route using the actual handler
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::any().map(move || mutant.clone()))
        .and(warp::any().map(move || tasks.clone()))
        .map(
            |ws: warp::ws::Ws, mutant_instance: Arc<MutAnt>, task_map: TaskMap| {
                ws.on_upgrade(move |socket| handlers::handle_ws(socket, mutant_instance, task_map))
            },
        );

    // Define server address
    // TODO: Make this configurable
    let addr: SocketAddr = ([127, 0, 0, 1], 3030).into();
    tracing::info!("WebSocket server listening on {}", addr);

    // Create a shutdown signal receiver
    let shutdown_signal = async {
        let ctrl_c = signal::ctrl_c();
        #[cfg(unix)]
        let sigterm = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install signal handler")
                .recv()
                .await;
        };
        #[cfg(not(unix))]
        let sigterm = std::future::pending::<()>(); // No SIGTERM on non-Unix

        tokio::select! {
            _ = ctrl_c => { tracing::info!("Received Ctrl+C, shutting down."); },
            _ = sigterm => { tracing::info!("Received SIGTERM, shutting down."); },
        }
    };

    // Start the server with graceful shutdown
    let (_, server) = warp::serve(ws_route).bind_with_graceful_shutdown(addr, shutdown_signal);

    tracing::info!("Starting server task...");
    // Await the server task. The lock file will be released when the process exits.
    tokio::task::spawn(server).await.map_err(Error::JoinError)?;
    tracing::info!("Server task finished.");

    // The lock file (_lock_file) is released automatically when the process exits
    // because the file descriptor associated with the lock is closed.
    Ok(())
}
