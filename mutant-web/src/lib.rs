#![feature(mapped_lock_guards)]

use std::{collections::HashMap, sync::{Arc, RwLock}};

use app::{context::init_context, window_system_mut, DEFAULT_WS_URL};
use futures::{channel::oneshot, StreamExt};
use log::{error, info};
use mutant_client::{MutantClient, ProgressReceiver};
use mutant_protocol::{TaskProgress, TaskResult};
use wasm_bindgen::prelude::*;

// mod app;
// mod cam_test;
// mod init;
// mod map;
// pub mod utils;

// use utils::{game, game_mut};

mod app;

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen(start)]
pub fn start() {
    init_panic_hook();
    wasm_logger::init(wasm_logger::Config::new(log::Level::Info));

    log::info!("Starting MutAnt Web Client");

    wasm_bindgen_futures::spawn_local(async move {
        async_start().await;
    });
}

pub async fn async_start() {
    init_context().await;
    run();

}

pub enum ClientRequest {
    Get(String, String, bool),
    Put(String, Vec<u8>, String, mutant_protocol::StorageMode, bool, bool),
    ListKeys,
    ListTasks
}

pub enum ClientResponse {
    Get(Result<TaskResult, String>),
    Put(Result<TaskResult, String>),
    ListKeys(Result<Vec<mutant_protocol::KeyDetails>, String>),
    ListTasks(Result<Vec<mutant_protocol::TaskListEntry>, String>),
}

pub struct Client {
    client: MutantClient,
    request_rx: futures::channel::mpsc::UnboundedReceiver<ClientRequest>,
}

impl Client {
    pub async fn spawn() -> ClientSender {
        info!("Spawning client");
        let (tx, rx) = futures::channel::mpsc::unbounded();

        let sender = ClientSender::new(tx);

        let responses = sender.responses.clone();

        spawn_local(async move {
            let mut this = Self {
                client: MutantClient::new(),
                request_rx: rx,
            };

            info!("Connecting to daemon");
            this.connect(DEFAULT_WS_URL).await.unwrap();
            info!("Client connected");

            while let Some(request) = this.request_rx.next().await {
                match request {
                    ClientRequest::Get(name, destination, public) => {
                        let result = this.get(&name, &destination, public).await;
                        let response_name = format!("get_{}_{}", name, destination);
                        if let Some(tx) = responses.write().unwrap().remove(&response_name) {
                            let _ = tx.send(ClientResponse::Get(result));
                        }
                    }
                    ClientRequest::Put(key, data, filename, mode, public, no_verify) => {
                        let result = this.put(&key, data, &filename, mode, public, no_verify).await;
                        let response_name = format!("put_{}_{}", key, filename);
                        if let Some(tx) = responses.write().unwrap().remove(&response_name) {
                            let _ = tx.send(ClientResponse::Put(result));
                        }
                    }
                    ClientRequest::ListKeys => {
                        let result = this.list_keys().await;
                        let response_name = "list_keys".to_string();
                        if let Some(tx) = responses.write().unwrap().remove(&response_name) {
                            let _ = tx.send(ClientResponse::ListKeys(result));
                        }
                    }
                    ClientRequest::ListTasks => {
                        let result = this.list_tasks().await;
                        let response_name = "list_tasks".to_string();
                        if let Some(tx) = responses.write().unwrap().remove(&response_name) {
                            let _ = tx.send(ClientResponse::ListTasks(result));
                        }
                    }
                }
            }
        });

        sender
    }

    async fn connect(&mut self, url: &str) -> Result<(), String> {
        self.client
            .connect(url)
            .await
            .map_err(|e| format!("{:?}", e))
    }

    pub async fn get(&mut self, name: &str, destination: &str, public: bool) -> Result<TaskResult, String> {
        match self.client.get(name, destination, public).await {
            Ok((task_future, progress_rx)) => {
                handle_get_progress(progress_rx);

                task_future.await.map_err(|e| format!("{:?}", e))
            }
            Err(e) => {
                error!("Failed to start get task: {:?}", e);
                Err(format!("{:?}", e))
            }
        }
    }

    pub async fn put(
        &mut self,
        key: &str,
        data: Vec<u8>,
        filename: &str,
        mode: mutant_protocol::StorageMode,
        public: bool,
        no_verify: bool,
    ) -> Result<TaskResult, String> {
        // Get the response name for this put operation
        let response_name = format!("put_{}_{}", key, filename);

        // Check if we have a progress object for this operation
        let progress_obj = {
            if let Some(sender) = Client::get_client_sender() {
                let put_progress = sender.put_progress.read().unwrap();
                put_progress.get(&response_name).cloned()
            } else {
                None
            }
        };

        match self.client.put_bytes(key, data, Some(filename.to_string()), mode, public, no_verify).await {
            Ok((task_future, progress_rx)) => {
                // If we have a progress object, update it based on the progress events
                if let Some(progress) = progress_obj {
                    // Spawn a task to update the progress object
                    spawn_local({
                        let mut progress_rx = progress_rx;
                        async move {
                            while let Some(progress_update) = progress_rx.recv().await {
                                match progress_update {
                                    Ok(TaskProgress::Put(event)) => {
                                        // Update the progress based on the event
                                        let mut progress_guard = progress.write().unwrap();
                                        match event {
                                            mutant_protocol::PutEvent::Starting {
                                                total_chunks,
                                                initial_written_count,
                                                initial_confirmed_count,
                                                chunks_to_reserve
                                            } => {
                                                // Initialize the operation
                                                progress_guard.operation.insert("put".to_string(), app::context::ProgressOperation {
                                                    nb_to_reserve: chunks_to_reserve,
                                                    nb_reserved: total_chunks - chunks_to_reserve,
                                                    total_pads: total_chunks,
                                                    nb_written: initial_written_count,
                                                    nb_confirmed: initial_confirmed_count,
                                                });
                                            },
                                            mutant_protocol::PutEvent::PadReserved => {
                                                // Update reserved count
                                                if let Some(op) = progress_guard.operation.get_mut("put") {
                                                    op.nb_reserved += 1;
                                                }
                                            },
                                            mutant_protocol::PutEvent::PadsWritten => {
                                                // Update written count
                                                if let Some(op) = progress_guard.operation.get_mut("put") {
                                                    op.nb_written += 1;
                                                }
                                            },
                                            mutant_protocol::PutEvent::PadsConfirmed => {
                                                // Update confirmed count
                                                if let Some(op) = progress_guard.operation.get_mut("put") {
                                                    op.nb_confirmed += 1;
                                                }
                                            },
                                            mutant_protocol::PutEvent::MultipartUploadProgress { bytes_uploaded, total_bytes } => {
                                                // Update multipart upload progress
                                                if let Some(op) = progress_guard.operation.get_mut("put") {
                                                    // Calculate progress as a percentage of total pads
                                                    let progress_percentage = bytes_uploaded as f32 / total_bytes as f32;
                                                    op.nb_reserved = (progress_percentage * op.total_pads as f32) as usize;
                                                    op.nb_written = op.nb_reserved;
                                                }
                                            },
                                            mutant_protocol::PutEvent::Complete => {
                                                // Mark operation as complete
                                                if let Some(op) = progress_guard.operation.get_mut("put") {
                                                    op.nb_reserved = op.total_pads;
                                                    op.nb_written = op.total_pads;
                                                    op.nb_confirmed = op.total_pads;
                                                }
                                            }
                                        }
                                    },
                                    _ => {
                                        // Ignore other progress types
                                    }
                                }
                            }
                        }
                    });
                } else {
                    // If we don't have a progress object, just log the progress
                    handle_put_progress(progress_rx);
                }

                // Wait for the task to complete and return the result
                task_future.await.map_err(|e| format!("{:?}", e))
            }
            Err(e) => {
                error!("Failed to start put task: {:?}", e);
                Err(format!("{:?}", e))
            }
        }
    }

    // Helper method to get the ClientSender instance
    fn get_client_sender() -> Option<Arc<ClientSender>> {
        // Use the context function directly
        let ctx = app::context::context();
        Some(ctx.get_client_sender())
    }

    pub async fn list_keys(&mut self) -> Result<Vec<mutant_protocol::KeyDetails>, String> {
        self.client.list_keys().await.map_err(|e| format!("{:?}", e))
    }

    pub async fn list_tasks(&mut self) -> Result<Vec<mutant_protocol::TaskListEntry>, String> {
        self.client.list_tasks().await.map_err(|e| format!("{:?}", e))
    }
}

pub struct ClientSender {
    tx: futures::channel::mpsc::UnboundedSender<ClientRequest>,
    responses: Arc<RwLock<HashMap<String, oneshot::Sender<ClientResponse>>>>,
    put_progress: Arc<RwLock<HashMap<String, Arc<RwLock<app::context::Progress>>>>>,
}

impl Clone for ClientSender {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            responses: self.responses.clone(),
            put_progress: self.put_progress.clone(),
        }
    }
}

impl ClientSender {
    pub fn new(tx: futures::channel::mpsc::UnboundedSender<ClientRequest>) -> Self {
        Self {
            tx,
            responses: Arc::new(RwLock::new(HashMap::new())),
            put_progress: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, name: String, destination: String, public: bool) -> Result<TaskResult, String> {
        let response_name = format!("get_{}_{}", name, destination);

        if self.responses.read().unwrap().contains_key(&response_name) {
            error!("Get request already pending for {}", response_name);
            return Err("Get request already pending".to_string());
        }

        let (tx, rx) = oneshot::channel();
        self.responses.write().unwrap().insert(response_name, tx);

        let _ = self.tx.unbounded_send(ClientRequest::Get(name, destination, public));

        rx.await.map(|result| {
            match result {
                ClientResponse::Get(Ok(result)) => Ok(result),
                ClientResponse::Get(Err(e)) => Err(e),
                _ => Err("Unexpected response".to_string()),
            }
        }).map_err(|e| format!("{:?}", e))?
    }

    pub async fn put(
        &self,
        key: String,
        data: Vec<u8>,
        filename: String,
        mode: mutant_protocol::StorageMode,
        public: bool,
        no_verify: bool,
        progress: Option<Arc<RwLock<app::context::Progress>>>,
    ) -> Result<TaskResult, String> {
        let response_name = format!("put_{}_{}", key, filename);

        if self.responses.read().unwrap().contains_key(&response_name) {
            error!("Put request already pending for {}", response_name);
            return Err("Put request already pending".to_string());
        }

        // Store the progress object if provided
        if let Some(progress_obj) = progress {
            let mut put_progress = self.put_progress.write().unwrap();
            put_progress.insert(response_name.clone(), progress_obj);
        }

        let (tx, rx) = oneshot::channel();
        self.responses.write().unwrap().insert(response_name.clone(), tx);

        let _ = self.tx.unbounded_send(ClientRequest::Put(key, data, filename, mode, public, no_verify));

        rx.await.map(|result| {
            match result {
                ClientResponse::Put(Ok(result)) => Ok(result),
                ClientResponse::Put(Err(e)) => Err(e),
                _ => Err("Unexpected response".to_string()),
            }
        }).map_err(|e| format!("{:?}", e))?
    }

    pub async fn list_keys(&self) -> Result<Vec<mutant_protocol::KeyDetails>, String> {
        let response_name = "list_keys".to_string();

        if self.responses.read().unwrap().contains_key(&response_name) {
            error!("ListKeys request already pending");
            return Err("ListKeys request already pending".to_string());
        }

        let (tx, rx) = oneshot::channel();

        self.responses.write().unwrap().insert(response_name, tx);

        let _ = self.tx.unbounded_send(ClientRequest::ListKeys);

        rx.await.map(|result| {
            match result {
                ClientResponse::ListKeys(Ok(result)) => Ok(result),
                ClientResponse::ListKeys(Err(e)) => Err(e),
                _ => Err("Unexpected response".to_string()),
            }
        }).map_err(|e| format!("{:?}", e))?
    }

    pub async fn list_tasks(&self) -> Result<Vec<mutant_protocol::TaskListEntry>, String> {
        let response_name = "list_tasks".to_string();

        if self.responses.read().unwrap().contains_key(&response_name) {
            error!("ListTasks request already pending");
            return Err("ListTasks request already pending".to_string());
        }

        let (tx, rx) = oneshot::channel();

        self.responses.write().unwrap().insert(response_name, tx);

        let _ = self.tx.unbounded_send(ClientRequest::ListTasks);

        rx.await.map(|result| {
            match result {
                ClientResponse::ListTasks(Ok(result)) => Ok(result),
                ClientResponse::ListTasks(Err(e)) => Err(e),
                _ => Err("Unexpected response".to_string()),
            }
        }).map_err(|e| format!("{:?}", e))?
    }
}

fn handle_get_progress(mut progress_rx: ProgressReceiver) {
    spawn_local(async move {
        while let Some(progress) = progress_rx.recv().await {
            match progress {
                Ok(progress) => {
                    log::info!("Progress: {:?}", progress);
                }
                Err(e) => {
                    error!("Progress error: {:?}", e);
                    break;
                }
            }
        }
    });
}

fn handle_put_progress(mut progress_rx: ProgressReceiver) {
    // Simply log the progress for now
    spawn_local(async move {
        while let Some(progress) = progress_rx.recv().await {
            match progress {
                Ok(progress) => {
                    log::info!("Progress: {:?}", progress);
                }
                Err(e) => {
                    error!("Progress error: {:?}", e);
                    break;
                }
            }
        }
    });
}

use eframe::egui;
use wasm_bindgen_futures::spawn_local;

struct MyApp {
    name: String,
    age: u32,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            name: "Arthur".to_owned(),
            age: 42,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            window_system_mut().draw(ui);
        });
    }
}

pub fn run() {
    use wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("canvas")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        app::init().await;

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(MyApp::default()))),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}
