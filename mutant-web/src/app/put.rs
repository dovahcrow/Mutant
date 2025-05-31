use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, RichText};
use js_sys::Uint8Array;
use mutant_protocol::StorageMode;
use serde::{Deserialize, Serialize};

use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::spawn_local;
use web_sys::{File, FileReader, Event};
use web_time::{Duration, SystemTime};

use super::Window;
use super::components::progress::detailed_progress;
use super::context;
use super::notifications;
use super::theme::{MutantColors, primary_button, success_button};

#[derive(Clone, Serialize, Deserialize)]
pub struct PutWindow {
    // File selection state
    selected_file: Arc<RwLock<Option<String>>>,
    file_size: Arc<RwLock<Option<u64>>>,
    #[serde(skip)] // Skip serializing file data to avoid localStorage quota issues
    file_data: Arc<RwLock<Option<Vec<u8>>>>,


    // Key name input
    key_name: Arc<RwLock<String>>,

    // Configuration options
    public: Arc<RwLock<bool>>,
    storage_mode: Arc<RwLock<StorageMode>>,
    no_verify: Arc<RwLock<bool>>,

    // File reading progress (Phase 1: File-to-Web)
    is_reading_file: Arc<RwLock<bool>>,
    file_read_progress: Arc<RwLock<f32>>,
    file_read_bytes: Arc<RwLock<u64>>,

    // Upload progress (Phase 2: Web-to-Daemon)
    is_uploading_to_daemon: Arc<RwLock<bool>>,
    daemon_upload_progress: Arc<RwLock<f32>>,
    daemon_upload_bytes: Arc<RwLock<u64>>,

    // Network progress tracking (Phase 3: Daemon-to-Network)
    reservation_progress: Arc<RwLock<f32>>,
    upload_progress: Arc<RwLock<f32>>,
    confirmation_progress: Arc<RwLock<f32>>,

    total_chunks: Arc<RwLock<usize>>,
    chunks_to_reserve: Arc<RwLock<usize>>,
    initial_written_count: Arc<RwLock<usize>>,
    initial_confirmed_count: Arc<RwLock<usize>>,

    // Upload state
    is_uploading: Arc<RwLock<bool>>,
    upload_complete: Arc<RwLock<bool>>,
    public_address: Arc<RwLock<Option<String>>>,
    error_message: Arc<RwLock<Option<String>>>,

    // Timing
    start_time: Arc<RwLock<Option<SystemTime>>>,
    elapsed_time: Arc<RwLock<Duration>>,
    last_progress_check: Arc<RwLock<SystemTime>>,

    // Progress tracking ID
    current_put_id: Arc<RwLock<Option<String>>>,
}

impl Default for PutWindow {
    fn default() -> Self {
        Self {
            selected_file: Arc::new(RwLock::new(None)),
            file_size: Arc::new(RwLock::new(None)),
            file_data: Arc::new(RwLock::new(None)),
            key_name: Arc::new(RwLock::new(String::new())),
            public: Arc::new(RwLock::new(false)),
            storage_mode: Arc::new(RwLock::new(StorageMode::Heaviest)),
            no_verify: Arc::new(RwLock::new(false)),

            // File reading progress (Phase 1)
            is_reading_file: Arc::new(RwLock::new(false)),
            file_read_progress: Arc::new(RwLock::new(0.0)),
            file_read_bytes: Arc::new(RwLock::new(0)),

            // Upload progress (Phase 2)
            is_uploading_to_daemon: Arc::new(RwLock::new(false)),
            daemon_upload_progress: Arc::new(RwLock::new(0.0)),
            daemon_upload_bytes: Arc::new(RwLock::new(0)),

            // Network progress (Phase 3)
            reservation_progress: Arc::new(RwLock::new(0.0)),
            upload_progress: Arc::new(RwLock::new(0.0)),
            confirmation_progress: Arc::new(RwLock::new(0.0)),
            total_chunks: Arc::new(RwLock::new(0)),
            chunks_to_reserve: Arc::new(RwLock::new(0)),
            initial_written_count: Arc::new(RwLock::new(0)),
            initial_confirmed_count: Arc::new(RwLock::new(0)),
            is_uploading: Arc::new(RwLock::new(false)),
            upload_complete: Arc::new(RwLock::new(false)),
            public_address: Arc::new(RwLock::new(None)),
            error_message: Arc::new(RwLock::new(None)),
            start_time: Arc::new(RwLock::new(None)),
            elapsed_time: Arc::new(RwLock::new(Duration::from_secs(0))),
            last_progress_check: Arc::new(RwLock::new(SystemTime::now())),
            current_put_id: Arc::new(RwLock::new(None)),
        }
    }
}

impl Window for PutWindow {
    fn name(&self) -> String {
        "MutAnt Upload".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        // Log that we're drawing the window
        log::debug!("Drawing PutWindow");

        // If we're uploading, check the progress before drawing the form
        // This ensures we have the latest progress values when drawing
        if *self.is_uploading.read().unwrap() && !*self.upload_complete.read().unwrap() {
            log::debug!("Upload in progress, checking progress");
            self.check_progress();
        } else {
            log::debug!("Upload not in progress: is_uploading={}, upload_complete={}",
                *self.is_uploading.read().unwrap(),
                *self.upload_complete.read().unwrap());
        }

        // Draw the form with the updated progress values
        self.draw_upload_form(ui);

        // Request a repaint to ensure we update frequently
        // This is crucial for smooth progress bar updates
        // Use a shorter interval (16ms = ~60fps) for smoother updates
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl PutWindow {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_progress(&self) {
        // Check if we should update the progress based on the timer
        let now = SystemTime::now();
        let last_check = *self.last_progress_check.read().unwrap();

        // Only check progress every 50ms to avoid excessive updates but still be responsive
        let should_check = match now.duration_since(last_check) {
            Ok(duration) => duration.as_millis() >= 50,
            Err(_) => true, // If there's an error, just check anyway
        };

        if !should_check {
            return;
        }

        // Update the last check time
        *self.last_progress_check.write().unwrap() = now;

        // Get the current put ID
        let put_id_opt = self.current_put_id.read().unwrap().clone();

        log::info!("UI: Checking progress for put ID: {:?}", put_id_opt);

        if let Some(put_id) = put_id_opt {
            // Get the context
            let ctx = context::context();

            // Get the progress for this put operation
            if let Some(progress) = ctx.get_put_progress(&put_id) {
                // Read the progress
                let progress_guard = progress.read().unwrap();

                // Check if we have an operation
                if let Some(op) = progress_guard.operation.get("put") {

                    // Update UI based on progress
                    let total_chunks = op.total_pads;
                    let reserved_count = op.nb_reserved;
                    let written_count = op.nb_written;
                    let confirmed_count = op.nb_confirmed;

                    // Store the chunks to reserve
                    *self.chunks_to_reserve.write().unwrap() = op.nb_to_reserve;
                    *self.initial_written_count.write().unwrap() = op.nb_written;
                    *self.initial_confirmed_count.write().unwrap() = op.nb_confirmed;

                    // Calculate progress percentages
                    let reservation_progress = if total_chunks > 0 {
                        reserved_count as f32 / total_chunks as f32
                    } else {
                        0.0
                    };

                    let upload_progress = if total_chunks > 0 {
                        written_count as f32 / total_chunks as f32
                    } else {
                        0.0
                    };

                    let confirmation_progress = if total_chunks > 0 {
                        confirmed_count as f32 / total_chunks as f32
                    } else {
                        0.0
                    };

                    // Update progress bars
                    *self.reservation_progress.write().unwrap() = reservation_progress;
                    *self.upload_progress.write().unwrap() = upload_progress;
                    *self.confirmation_progress.write().unwrap() = confirmation_progress;

                    // Update total chunks
                    *self.total_chunks.write().unwrap() = total_chunks;

                    // Check if operation is complete
                    if confirmed_count == total_chunks && total_chunks > 0 {
                        // Mark upload as complete
                        *self.is_uploading.write().unwrap() = false;
                        *self.upload_complete.write().unwrap() = true;

                        // Set elapsed time
                        if let Some(start) = *self.start_time.read().unwrap() {
                            *self.elapsed_time.write().unwrap() = start.elapsed().unwrap();
                        }

                        // Show notification
                        notifications::info("Upload complete!".to_string());

                        // Refresh the keys list to update the file explorer
                        spawn_local(async {
                            let ctx = context::context();
                            let _ = ctx.list_keys().await;
                        });

                        // If this is a public upload, get the key details to find the public address
                        let public = *self.public.read().unwrap();
                        if public {
                            // Fetch the key details to get the public address
                            spawn_local({
                                let ctx = context::context();
                                let key_name = self.key_name.read().unwrap().clone();
                                let public_address_clone = self.public_address.clone();

                                async move {
                                    // Fetch the keys directly
                                    let keys = ctx.list_keys().await;

                                    // Find our key
                                    for key in keys {
                                        if key.key == key_name && key.is_public {
                                            if let Some(addr) = key.public_address {
                                                *public_address_clone.write().unwrap() = Some(addr);
                                                break;
                                            }
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
    }



    // TRUE streaming upload - reads file chunks and sends them directly to daemon
    fn start_streaming_upload_with_file(
        file: File,
        key_name: String,
        filename: String,
        file_size: f64,
        storage_mode: mutant_protocol::StorageMode,
        public: bool,
        no_verify: bool,
        current_put_id: Arc<RwLock<Option<String>>>,
        error_message: Arc<RwLock<Option<String>>>,
        is_uploading: Arc<RwLock<bool>>,
        is_uploading_to_daemon: Arc<RwLock<bool>>,
        daemon_upload_progress: Arc<RwLock<f32>>,
        daemon_upload_bytes: Arc<RwLock<u64>>,
    ) {


        spawn_local(async move {
            let ctx = context::context();
            let file_size_u64 = file_size as u64;

            // Step 1: Initialize streaming put with daemon
            match ctx.put_streaming_init(&key_name, file_size_u64, &filename, storage_mode, public, no_verify).await {
                Ok(task_id) => {
                    // Store the task ID for progress tracking
                    let task_id_string = task_id.to_string();
                    *current_put_id.write().unwrap() = Some(task_id_string);

                    // Step 2: Stream file chunks directly to daemon
                    const CHUNK_SIZE: u64 = 256 * 1024; // 256KB chunks
                    const DELAY_MS: u64 = 10; // Small delay between chunks

                    let mut offset = 0u64;
                    let mut chunk_index = 0usize;
                    let total_chunks = ((file_size_u64 + CHUNK_SIZE - 1) / CHUNK_SIZE) as usize;



                    while offset < file_size_u64 {
                        let chunk_size = std::cmp::min(CHUNK_SIZE, file_size_u64 - offset);
                        let end_offset = offset + chunk_size;
                        let is_last = end_offset >= file_size_u64;



                        // Create a blob slice for this chunk
                        let blob_slice = match file.slice_with_f64_and_f64(offset as f64, end_offset as f64) {
                            Ok(slice) => slice,
                            Err(e) => {
                                log::error!("Failed to slice file: {:?}", e);
                                *error_message.write().unwrap() = Some("Failed to read file chunk".to_string());
                                *is_uploading.write().unwrap() = false;
                                *is_uploading_to_daemon.write().unwrap() = false;
                                notifications::error("Failed to read file chunk".to_string());
                                return;
                            }
                        };

                        // Read this chunk
                        match Self::read_file_chunk_async(blob_slice).await {
                            Ok(chunk_data) => {
                                // Send this chunk directly to daemon
                                match ctx.put_streaming_chunk(task_id, chunk_index, total_chunks, chunk_data, is_last).await {
                                    Ok(_) => {
                                        offset = end_offset;
                                        chunk_index += 1;

                                        // Update daemon upload progress
                                        let progress = offset as f32 / file_size_u64 as f32;
                                        *daemon_upload_progress.write().unwrap() = progress;
                                        *daemon_upload_bytes.write().unwrap() = offset;

                                        // Small delay to keep UI responsive
                                        if !is_last {
                                            let start = web_time::SystemTime::now();
                                            while start.elapsed().unwrap().as_millis() < DELAY_MS as u128 {
                                                // Busy wait for a short time
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        log::error!("Failed to send chunk {} to daemon: {}", chunk_index, e);
                                        *error_message.write().unwrap() = Some(format!("Upload failed: {}", e));
                                        *is_uploading.write().unwrap() = false;
                                        *is_uploading_to_daemon.write().unwrap() = false;
                                        notifications::error(format!("Upload failed: {}", e));
                                        return;
                                    }
                                }
                            },
                            Err(e) => {
                                log::error!("Error reading file chunk {}: {}", chunk_index, e);
                                *error_message.write().unwrap() = Some(format!("Error reading file: {}", e));
                                *is_uploading.write().unwrap() = false;
                                *is_uploading_to_daemon.write().unwrap() = false;
                                notifications::error(format!("Error reading file: {}", e));
                                return;
                            }
                        }
                    }

                    *is_uploading_to_daemon.write().unwrap() = false;

                    // Final progress update - 100% complete
                    *daemon_upload_progress.write().unwrap() = 1.0;
                    *daemon_upload_bytes.write().unwrap() = file_size_u64;

                    notifications::info("File upload to daemon completed!".to_string());

                    // Refresh the keys list to update the file explorer
                    spawn_local(async {
                        let ctx = context::context();
                        let _ = ctx.list_keys().await;
                    });
                },
                Err(e) => {
                    log::error!("Failed to initialize streaming upload: {}", e);
                    *error_message.write().unwrap() = Some(format!("Failed to start upload: {}", e));
                    *is_uploading.write().unwrap() = false;
                    *is_uploading_to_daemon.write().unwrap() = false;
                    notifications::error(format!("Failed to start upload: {}", e));
                }
            }
        });
    }

    // Helper function to read a single file chunk asynchronously
    async fn read_file_chunk_async(blob: web_sys::Blob) -> Result<Vec<u8>, String> {
        // Create a FileReader for this chunk
        let reader = FileReader::new().map_err(|_| "Failed to create FileReader")?;

        // Create a promise that resolves when the chunk is read
        let (tx, rx) = futures::channel::oneshot::channel();
        let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));

        // Set up onload handler
        let reader_clone = reader.clone();
        let tx_clone = tx.clone();
        let onload = Closure::once(move |_event: Event| {
            if let Some(sender) = tx_clone.borrow_mut().take() {
                match reader_clone.result() {
                    Ok(array_buffer) => {
                        let array = Uint8Array::new(&array_buffer);
                        let mut data = vec![0; array.length() as usize];
                        array.copy_to(&mut data);
                        let _ = sender.send(Ok(data));
                    },
                    Err(_) => {
                        let _ = sender.send(Err("Failed to read chunk".to_string()));
                    }
                }
            }
        });

        // Set up onerror handler
        let tx_error = tx.clone();
        let onerror = Closure::once(move |_event: Event| {
            if let Some(sender) = tx_error.borrow_mut().take() {
                let _ = sender.send(Err("Error reading chunk".to_string()));
            }
        });

        reader.set_onload(Some(onload.as_ref().unchecked_ref()));
        reader.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        // Start reading the chunk
        reader.read_as_array_buffer(&blob).map_err(|_| "Failed to start reading chunk")?;

        // Don't forget the closures
        onload.forget();
        onerror.forget();

        // Wait for the result
        rx.await.map_err(|_| "Channel error".to_string())?
    }

    fn start_upload(&self) {

        // Get upload parameters
        let key_name = self.key_name.read().unwrap().clone();
        let public = *self.public.read().unwrap();
        let storage_mode = self.storage_mode.read().unwrap().clone();
        let no_verify = *self.no_verify.read().unwrap();

        if key_name.is_empty() {
            notifications::error("Please enter a key name".to_string());
            return;
        }

        // Set upload state
        *self.is_uploading.write().unwrap() = true;
        *self.upload_complete.write().unwrap() = false;
        *self.start_time.write().unwrap() = Some(SystemTime::now());

        // Reset all progress
        *self.is_uploading_to_daemon.write().unwrap() = true; // We're streaming to daemon
        *self.daemon_upload_progress.write().unwrap() = 0.0;
        *self.daemon_upload_bytes.write().unwrap() = 0;
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;



        // Directly trigger file selection for upload
        self.select_file_and_upload(key_name, storage_mode, public, no_verify);
    }

    fn select_file_and_upload(
        &self,
        key_name: String,
        storage_mode: mutant_protocol::StorageMode,
        public: bool,
        no_verify: bool,
    ) {


        // Create file input element
        let document = web_sys::window().unwrap().document().unwrap();
        let input = document
            .create_element("input")
            .unwrap()
            .dyn_into::<web_sys::HtmlInputElement>()
            .unwrap();

        input.set_type("file");
        input.set_accept("*/*");

        // Clone references for the closure
        let current_put_id = self.current_put_id.clone();
        let error_message = self.error_message.clone();
        let is_uploading = self.is_uploading.clone();
        let is_uploading_to_daemon = self.is_uploading_to_daemon.clone();
        let daemon_upload_progress = self.daemon_upload_progress.clone();
        let daemon_upload_bytes = self.daemon_upload_bytes.clone();
        let selected_file = self.selected_file.clone();
        let file_size = self.file_size.clone();
        let file_data = self.file_data.clone();
        let input_clone = input.clone();

        let onchange = Closure::once(move |_event: Event| {
            if let Some(files) = input_clone.files() {
                if files.length() > 0 {
                    if let Some(file) = files.get(0) {
                        let file_name = file.name();
                        let file_size_js = file.size();



                        // Update state with selected file info
                        *selected_file.write().unwrap() = Some(file_name.clone());
                        *file_size.write().unwrap() = Some(file_size_js as u64);
                        *file_data.write().unwrap() = Some(Vec::new()); // Mark as selected

                        // Start the actual streaming upload immediately
                        Self::start_streaming_upload_with_file(
                            file,
                            key_name,
                            file_name,
                            file_size_js,
                            storage_mode,
                            public,
                            no_verify,
                            current_put_id,
                            error_message,
                            is_uploading,
                            is_uploading_to_daemon,
                            daemon_upload_progress,
                            daemon_upload_bytes,
                        );
                    }
                } else {
                    // User cancelled file selection
                    *is_uploading.write().unwrap() = false;
                    *is_uploading_to_daemon.write().unwrap() = false;
                }
            }
        });

        input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
        onchange.forget();

        // Trigger file picker
        input.click();
    }





    fn reset(&self) {
        // Reset all state for a new upload
        *self.selected_file.write().unwrap() = None;
        *self.file_size.write().unwrap() = None;
        *self.file_data.write().unwrap() = None;
        *self.key_name.write().unwrap() = String::new();

        // Reset file reading progress (Phase 1)
        *self.is_reading_file.write().unwrap() = false;
        *self.file_read_progress.write().unwrap() = 0.0;
        *self.file_read_bytes.write().unwrap() = 0;

        // Reset upload progress (Phase 2)
        *self.is_uploading_to_daemon.write().unwrap() = false;
        *self.daemon_upload_progress.write().unwrap() = 0.0;
        *self.daemon_upload_bytes.write().unwrap() = 0;

        // Reset network progress (Phase 3)
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;
        *self.is_uploading.write().unwrap() = false;
        *self.upload_complete.write().unwrap() = false;
        *self.public_address.write().unwrap() = None;
        *self.error_message.write().unwrap() = None;
        *self.start_time.write().unwrap() = None;
        *self.elapsed_time.write().unwrap() = Duration::from_secs(0);
        *self.current_put_id.write().unwrap() = None;
    }

    fn draw_upload_form(&mut self, ui: &mut egui::Ui) {
        let is_uploading = *self.is_uploading.read().unwrap();
        let upload_complete = *self.upload_complete.read().unwrap();
        let is_reading_file = *self.is_reading_file.read().unwrap();

        // Check if we're in any kind of processing state
        let is_processing = is_uploading || is_reading_file;

        if !is_processing && !upload_complete {
            // File selection section
            ui.heading(RichText::new("📤 Upload File").size(20.0).color(MutantColors::TEXT_PRIMARY));
            ui.add_space(15.0);

            // Key name input with styled frame
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Key Name:").color(MutantColors::TEXT_PRIMARY));
                    ui.add_space(5.0);
                    let mut key_name = self.key_name.write().unwrap();
                    ui.text_edit_singleline(&mut *key_name);
                });
            });

            ui.add_space(10.0);

            // Show selected file info if any (from previous upload)
            if let Some(filename) = &*self.selected_file.read().unwrap() {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Last Selected File:").color(MutantColors::TEXT_SECONDARY));
                        ui.label(RichText::new(filename).color(MutantColors::ACCENT_BLUE));

                        if let Some(size) = *self.file_size.read().unwrap() {
                            ui.label(RichText::new(format!("Size: {} bytes", size)).color(MutantColors::TEXT_MUTED));
                        }
                    });
                });
                ui.add_space(10.0);
            }

            ui.add_space(10.0);

            // Configuration options
            ui.collapsing("Upload Options", |ui| {
                // Public checkbox
                ui.horizontal(|ui| {
                    let mut public = self.public.write().unwrap();
                    ui.checkbox(&mut *public, "Public");
                    ui.label("Make this file publicly accessible");
                });

                // Storage mode selection
                ui.horizontal(|ui| {
                    ui.label("Storage Mode:");
                    let mut storage_mode = self.storage_mode.write().unwrap();

                    egui::ComboBox::new("storage_mode", "")
                        .selected_text(format!("{:?}", *storage_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut *storage_mode, StorageMode::Light, "Light");
                            ui.selectable_value(&mut *storage_mode, StorageMode::Medium, "Medium");
                            ui.selectable_value(&mut *storage_mode, StorageMode::Heavy, "Heavy");
                            ui.selectable_value(&mut *storage_mode, StorageMode::Heaviest, "Heaviest");
                        });
                });

                // No verify checkbox
                ui.horizontal(|ui| {
                    let mut no_verify = self.no_verify.write().unwrap();
                    ui.checkbox(&mut *no_verify, "Skip Verification");
                    ui.label("Skip verification of uploaded data (faster but less safe)");
                });
            });

            ui.add_space(10.0);

            // Upload button - now triggers file selection and upload
            let can_upload = !self.key_name.read().unwrap().is_empty();

            ui.add_space(15.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(can_upload, primary_button("📁 Select File and Upload")).clicked() {
                    self.start_upload();
                }
            });

            if !can_upload {
                ui.add_space(5.0);
                ui.label(RichText::new("⚠ Please enter a key name").color(MutantColors::WARNING));
            }

            // Show error message if any
            if let Some(error) = &*self.error_message.read().unwrap() {
                ui.add_space(10.0);
                ui.group(|ui| {
                    ui.label(RichText::new(format!("❌ Error: {}", error)).color(MutantColors::ERROR));
                });
            }
        } else if is_reading_file {
            // File reading progress section
            ui.heading("Reading File");
            ui.add_space(10.0);

            let file_name = self.selected_file.read().unwrap().clone().unwrap_or_default();
            ui.label(format!("Reading: {}", file_name));

            ui.add_space(5.0);

            // File reading progress
            let file_read_progress = *self.file_read_progress.read().unwrap();
            let file_read_bytes = *self.file_read_bytes.read().unwrap();
            let file_size = self.file_size.read().unwrap().unwrap_or(0);

            ui.label("Loading file into memory:");
            ui.add(detailed_progress(
                file_read_progress,
                file_read_bytes as usize,
                file_size as usize,
                "Reading...".to_string()
            ));

            ui.add_space(10.0);
            ui.label(RichText::new("Please wait while the file is being loaded...").color(Color32::GRAY));
        } else if is_uploading {
            // Progress section
            ui.heading("Upload Progress");
            ui.add_space(10.0);

            let key_name = self.key_name.read().unwrap();
            ui.label(format!("Uploading: {}", *key_name));

            ui.add_space(5.0);

            // Check if we're in daemon upload phase
            let is_uploading_to_daemon = *self.is_uploading_to_daemon.read().unwrap();
            if is_uploading_to_daemon {
                // Show daemon upload progress
                let daemon_upload_progress = *self.daemon_upload_progress.read().unwrap();
                let daemon_upload_bytes = *self.daemon_upload_bytes.read().unwrap();
                let file_size = self.file_size.read().unwrap().unwrap_or(0);

                ui.label("Sending to daemon:");
                ui.add(detailed_progress(
                    daemon_upload_progress,
                    daemon_upload_bytes as usize,
                    file_size as usize,
                    "Uploading...".to_string()
                ));

                ui.add_space(5.0);
            }

            // Calculate elapsed time
            let elapsed = if let Some(start_time) = *self.start_time.read().unwrap() {
                start_time.elapsed().unwrap()
            } else {
                *self.elapsed_time.read().unwrap()
            };

            let elapsed_str = format_elapsed_time(elapsed);

            // Get total chunks and current progress
            let total_chunks = *self.total_chunks.read().unwrap();
            let chunks_to_reserve = *self.chunks_to_reserve.read().unwrap();

            // Calculate current counts based on progress
            let reservation_progress = *self.reservation_progress.read().unwrap();
            let upload_progress = *self.upload_progress.read().unwrap();
            let confirmation_progress = *self.confirmation_progress.read().unwrap();

            // Get the latest progress values directly from the context
            let (reservation_progress, upload_progress, confirmation_progress, total_chunks) = {
                if let Some(put_id) = &*self.current_put_id.read().unwrap() {
                    let ctx = context::context();
                    if let Some(progress) = ctx.get_put_progress(put_id) {
                        let progress_guard = progress.read().unwrap();
                        if let Some(op) = progress_guard.operation.get("put") {
                            // Calculate progress percentages
                            let res_progress = if op.total_pads > 0 {
                                op.nb_reserved as f32 / op.total_pads as f32
                            } else {
                                0.0
                            };

                            let up_progress = if op.total_pads > 0 {
                                op.nb_written as f32 / op.total_pads as f32
                            } else {
                                0.0
                            };

                            let conf_progress = if op.total_pads > 0 {
                                op.nb_confirmed as f32 / op.total_pads as f32
                            } else {
                                0.0
                            };

                            // Update the stored progress values
                            *self.reservation_progress.write().unwrap() = res_progress;
                            *self.upload_progress.write().unwrap() = up_progress;
                            *self.confirmation_progress.write().unwrap() = conf_progress;
                            *self.total_chunks.write().unwrap() = op.total_pads;

                            (res_progress, up_progress, conf_progress, op.total_pads)
                        } else {
                            (reservation_progress, upload_progress, confirmation_progress, total_chunks)
                        }
                    } else {
                        (reservation_progress, upload_progress, confirmation_progress, total_chunks)
                    }
                } else {
                    (reservation_progress, upload_progress, confirmation_progress, total_chunks)
                }
            };

            // Calculate current counts for each stage
            let reserved_count = if chunks_to_reserve > 0 {
                (reservation_progress * total_chunks as f32) as usize
            } else {
                (reservation_progress * total_chunks as f32) as usize
            };

            let uploaded_count = (upload_progress * total_chunks as f32) as usize;
            let confirmed_count = (confirmation_progress * total_chunks as f32) as usize;

            log::debug!("Drawing progress bars: reservation={:.2}%, upload={:.2}%, confirmation={:.2}%",
                reservation_progress * 100.0, upload_progress * 100.0, confirmation_progress * 100.0);

            // Reservation progress bar
            ui.label("Reserving pads:");
            ui.add(detailed_progress(reservation_progress, reserved_count, total_chunks, elapsed_str.clone()));

            ui.add_space(5.0);

            // Upload progress bar
            ui.label("Uploading pads:");
            ui.add(detailed_progress(upload_progress, uploaded_count, total_chunks, elapsed_str.clone()));

            ui.add_space(5.0);

            // Confirmation progress bar
            ui.label("Confirming pads:");
            ui.add(detailed_progress(confirmation_progress, confirmed_count, total_chunks, elapsed_str));

            // Cancel button
            ui.add_space(10.0);
            if ui.button("Cancel").clicked() {
                // TODO: Implement cancellation
                *self.is_uploading.write().unwrap() = false;
                notifications::warning("Upload cancelled".to_string());
            }
        } else if upload_complete {
            // Upload complete section
            ui.heading("Upload Complete");
            ui.add_space(10.0);

            let key_name = self.key_name.read().unwrap();
            ui.label(format!("Successfully uploaded: {}", *key_name));

            // Show elapsed time
            let elapsed = *self.elapsed_time.read().unwrap();
            ui.label(format!("Time taken: {}", format_elapsed_time(elapsed)));

            // Show public address if available
            if let Some(address) = &*self.public_address.read().unwrap() {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Public index address:");
                    ui.text_edit_singleline(&mut address.clone());
                });
            }

            ui.add_space(15.0);
            ui.horizontal(|ui| {
                if ui.add(success_button("📤 Upload Another File")).clicked() {
                    self.reset();
                }
            });
        }
    }
}

// Helper function to format elapsed time (similar to CLI implementation)
fn format_elapsed_time(duration: Duration) -> String {
    let total_seconds = duration.as_secs();

    if total_seconds < 60 {
        format!("{}s", total_seconds)
    } else if total_seconds < 3600 {
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{}m {}s", minutes, seconds)
    } else {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        format!("{}h {}m {}s", hours, minutes, seconds)
    }
}