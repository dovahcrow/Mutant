use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, RichText};
use js_sys::Uint8Array;
use mutant_protocol::{StorageMode, TaskProgress, PutEvent};
use serde::{Deserialize, Serialize};

use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::spawn_local;
use web_sys::{FileReader, HtmlInputElement, HtmlElement, Event};
use web_time::{Duration, SystemTime};

use super::Window;
use super::components::progress::detailed_progress;
use super::context;
use super::notifications;

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

    // Progress tracking
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
}

impl Default for PutWindow {
    fn default() -> Self {
        Self {
            selected_file: Arc::new(RwLock::new(None)),
            file_size: Arc::new(RwLock::new(None)),
            file_data: Arc::new(RwLock::new(None)),
            key_name: Arc::new(RwLock::new(String::new())),
            public: Arc::new(RwLock::new(false)),
            storage_mode: Arc::new(RwLock::new(StorageMode::Medium)),
            no_verify: Arc::new(RwLock::new(false)),
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
        }
    }
}

impl Window for PutWindow {
    fn name(&self) -> String {
        "MutAnt Upload".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.draw_upload_form(ui);
    }
}

impl PutWindow {
    pub fn new() -> Self {
        Self::default()
    }

    fn draw_upload_form(&mut self, ui: &mut egui::Ui) {
        let is_uploading = *self.is_uploading.read().unwrap();
        let upload_complete = *self.upload_complete.read().unwrap();

        if !is_uploading && !upload_complete {
            // File selection section
            ui.heading("Upload File");
            ui.add_space(10.0);

            // Key name input
            ui.horizontal(|ui| {
                ui.label("Key Name:");
                let mut key_name = self.key_name.write().unwrap();
                ui.text_edit_singleline(&mut *key_name);
            });

            ui.add_space(5.0);

            // File selection button
            if ui.button("Select File").clicked() {
                self.select_file();
            }

            // Show selected file info
            if let Some(filename) = &*self.selected_file.read().unwrap() {
                ui.horizontal(|ui| {
                    ui.label("Selected File:");
                    ui.label(filename);
                });

                if let Some(size) = *self.file_size.read().unwrap() {
                    ui.label(format!("Size: {} bytes", size));
                }
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

            // Upload button
            let can_upload = self.selected_file.read().unwrap().is_some() &&
                            !self.key_name.read().unwrap().is_empty() &&
                            self.file_data.read().unwrap().is_some();

            if ui.add_enabled(can_upload, egui::Button::new("Upload")).clicked() {
                self.start_upload();
            }

            if !can_upload {
                ui.label(RichText::new("Please select a file and enter a key name").color(Color32::LIGHT_RED));
            }

            // Show error message if any
            if let Some(error) = &*self.error_message.read().unwrap() {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Error: {}", error)).color(Color32::RED));
            }
        } else if is_uploading {
            // Progress section
            ui.heading("Upload Progress");
            ui.add_space(10.0);

            let key_name = self.key_name.read().unwrap();
            ui.label(format!("Uploading: {}", *key_name));

            ui.add_space(5.0);

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

            // Calculate current counts for each stage
            let reserved_count = if chunks_to_reserve > 0 {
                (reservation_progress * total_chunks as f32) as usize
            } else {
                total_chunks
            };

            let uploaded_count = (upload_progress * total_chunks as f32) as usize;
            let confirmed_count = (confirmation_progress * total_chunks as f32) as usize;

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

            ui.add_space(10.0);
            if ui.button("Upload Another File").clicked() {
                self.reset();
            }
        }
    }

    fn select_file(&self) {
        log::info!("File selection requested");

        // Get references to our state
        let selected_file = self.selected_file.clone();
        let file_size = self.file_size.clone();
        let file_data = self.file_data.clone();

        // Create a file input element
        let window = web_sys::window().expect("no global window exists");
        let document = window.document().expect("no document exists");

        let input: HtmlInputElement = document
            .create_element("input")
            .expect("failed to create input element")
            .dyn_into::<HtmlInputElement>()
            .expect("failed to cast to HtmlInputElement");

        // Set input attributes
        input.set_type("file");

        // Cast to HtmlElement to access style
        let input_html: &HtmlElement = input.dyn_ref::<HtmlElement>()
            .expect("input should be an HtmlElement");
        input_html.style().set_property("display", "none").expect("failed to set style");

        // Append to document body
        let body = document.body().expect("document should have a body");
        body.append_child(&input).expect("failed to append input to body");

        // Create onchange handler
        let onchange = Closure::once(move |event: Event| {
            let input: HtmlInputElement = event
                .target()
                .expect("event should have a target")
                .dyn_into::<HtmlInputElement>()
                .expect("target should be an HtmlInputElement");

            // Get the selected file
            let file_list = input.files().expect("input should have files");
            if let Some(file) = file_list.get(0) {
                let file_name = file.name();
                let file_size_js = file.size();

                // Update state with file name and size
                *selected_file.write().unwrap() = Some(file_name.clone());
                *file_size.write().unwrap() = Some(file_size_js as u64);

                // Read file content
                let reader = FileReader::new().expect("failed to create FileReader");
                let reader_clone = reader.clone();

                // Create onload handler for the reader
                let file_data_clone = file_data.clone();
                let file_name_clone = file_name.clone();

                let onload = Closure::once(move |_event: Event| {
                    // Get array buffer from reader
                    let array_buffer = reader_clone.result().expect("failed to get result");
                    let array = Uint8Array::new(&array_buffer);

                    // Convert to Rust Vec<u8>
                    let mut data = vec![0; array.length() as usize];
                    array.copy_to(&mut data);

                    // Update state with file data
                    *file_data_clone.write().unwrap() = Some(data);

                    // Show notification
                    notifications::info(format!("File selected: {}", file_name_clone));

                    // Remove the input element
                    if let Some(parent) = input.parent_node() {
                        parent.remove_child(&input).expect("failed to remove input");
                    }
                });

                // Set onload handler
                reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                onload.forget(); // Prevent closure from being dropped

                // Read the file as array buffer
                reader.read_as_array_buffer(&file).expect("failed to read file");
            }
        });

        // Set onchange handler
        input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
        onchange.forget(); // Prevent closure from being dropped

        // Click the input to open file dialog
        input.click();
    }

    fn start_upload(&self) {
        log::info!("Starting upload");

        // Set upload state
        *self.is_uploading.write().unwrap() = true;
        *self.upload_complete.write().unwrap() = false;
        *self.start_time.write().unwrap() = Some(SystemTime::now());

        // Reset progress
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;

        // Get upload parameters
        let key_name = self.key_name.read().unwrap().clone();
        let public = *self.public.read().unwrap();
        let storage_mode = self.storage_mode.read().unwrap().clone();
        let no_verify = *self.no_verify.read().unwrap();
        let file_data = self.file_data.read().unwrap().clone().unwrap_or_default();
        let filename = self.selected_file.read().unwrap().clone().unwrap_or_default();

        // Clone Arc references for the async task
        let reservation_progress = self.reservation_progress.clone();
        let upload_progress = self.upload_progress.clone();
        let confirmation_progress = self.confirmation_progress.clone();
        let total_chunks = self.total_chunks.clone();
        let chunks_to_reserve = self.chunks_to_reserve.clone();
        let initial_written_count = self.initial_written_count.clone();
        let initial_confirmed_count = self.initial_confirmed_count.clone();
        let is_uploading = self.is_uploading.clone();
        let upload_complete = self.upload_complete.clone();
        let public_address = self.public_address.clone();
        let error_message = self.error_message.clone();
        let elapsed_time = self.elapsed_time.clone();
        let start_time = self.start_time.clone();

        // Start the actual upload using the context
        spawn_local(async move {
            let ctx = context::context();

            match ctx.put(&key_name, file_data, &filename, storage_mode, public, no_verify).await {
                Ok((task_id, mut progress_rx)) => {
                    log::info!("Upload started with task ID: {}", task_id);

                    // Process progress updates
                    while let Some(progress) = progress_rx.recv().await {
                        match progress {
                            Ok(TaskProgress::Put(event)) => {
                                match event {
                                    PutEvent::Starting {
                                        total_chunks: tc,
                                        initial_written_count: iwc,
                                        initial_confirmed_count: icc,
                                        chunks_to_reserve: ctr
                                    } => {
                                        log::info!("Starting put operation - Total: {}, Written: {}, Confirmed: {}, To Reserve: {}",
                                            tc, iwc, icc, ctr);

                                        // Store the values
                                        *total_chunks.write().unwrap() = tc;
                                        *initial_written_count.write().unwrap() = iwc;
                                        *initial_confirmed_count.write().unwrap() = icc;
                                        *chunks_to_reserve.write().unwrap() = ctr;

                                        // Update progress bars
                                        if tc > 0 {
                                            // Calculate initial reservation progress
                                            let mut res_progress = reservation_progress.write().unwrap();
                                            if ctr > 0 {
                                                // If we have chunks to reserve, calculate based on how many are already reserved
                                                let reserved = tc - ctr;
                                                *res_progress = reserved as f32 / tc as f32;
                                                log::info!("Initial reservation progress: {}/{} = {}", reserved, tc, *res_progress);
                                            } else {
                                                // If no chunks to reserve, we're done with reservation
                                                *res_progress = 1.0;
                                                log::info!("No chunks to reserve, setting reservation progress to 1.0");
                                            }

                                            // Update upload progress based on written chunks
                                            let mut up_progress = upload_progress.write().unwrap();
                                            *up_progress = iwc as f32 / tc as f32;
                                            log::info!("Initial upload progress: {}/{} = {}", iwc, tc, *up_progress);

                                            // Update confirmation progress based on confirmed chunks
                                            let mut conf_progress = confirmation_progress.write().unwrap();
                                            *conf_progress = icc as f32 / tc as f32;
                                            log::info!("Initial confirmation progress: {}/{} = {}", icc, tc, *conf_progress);
                                        }
                                    },
                                    PutEvent::PadReserved => {
                                        log::info!("Pad reserved");
                                        let tc = *total_chunks.read().unwrap();
                                        if tc > 0 {
                                            let mut res_progress = reservation_progress.write().unwrap();
                                            if *res_progress < 1.0 {
                                                let new_progress = *res_progress + (1.0 / tc as f32);
                                                *res_progress = if new_progress > 1.0 { 1.0 } else { new_progress };
                                                log::info!("Updated reservation progress: {}", *res_progress);
                                            }
                                        }
                                    },
                                    PutEvent::PadsWritten => {
                                        log::info!("Pads written");
                                        let tc = *total_chunks.read().unwrap();
                                        if tc > 0 {
                                            let mut up_progress = upload_progress.write().unwrap();
                                            if *up_progress < 1.0 {
                                                let new_progress = *up_progress + (1.0 / tc as f32);
                                                *up_progress = if new_progress > 1.0 { 1.0 } else { new_progress };
                                                log::info!("Updated upload progress: {}", *up_progress);
                                            }
                                        }
                                    },
                                    PutEvent::PadsConfirmed => {
                                        log::info!("Pads confirmed");
                                        let tc = *total_chunks.read().unwrap();
                                        if tc > 0 {
                                            let mut conf_progress = confirmation_progress.write().unwrap();
                                            if *conf_progress < 1.0 {
                                                let new_progress = *conf_progress + (1.0 / tc as f32);
                                                *conf_progress = if new_progress > 1.0 { 1.0 } else { new_progress };
                                                log::info!("Updated confirmation progress: {}", *conf_progress);
                                            }
                                        }
                                    },
                                    PutEvent::MultipartUploadProgress { bytes_uploaded, total_bytes } => {
                                        log::info!("Multipart upload progress: {}/{} bytes", bytes_uploaded, total_bytes);

                                        // Update the upload progress bar based on bytes uploaded
                                        let mut up_progress = upload_progress.write().unwrap();
                                        *up_progress = bytes_uploaded as f32 / total_bytes as f32;

                                        // Also update the reservation progress to show we're making progress
                                        // For multipart uploads, we'll just set reservation to match upload progress
                                        let mut res_progress = reservation_progress.write().unwrap();
                                        *res_progress = *up_progress;
                                    },
                                    PutEvent::Complete => {
                                        log::info!("Upload complete");

                                        // Set all progress bars to 100%
                                        *reservation_progress.write().unwrap() = 1.0;
                                        *upload_progress.write().unwrap() = 1.0;
                                        *confirmation_progress.write().unwrap() = 1.0;

                                        // Mark upload as complete
                                        *is_uploading.write().unwrap() = false;
                                        *upload_complete.write().unwrap() = true;

                                        // Set elapsed time
                                        if let Some(start) = *start_time.read().unwrap() {
                                            *elapsed_time.write().unwrap() = start.elapsed().unwrap();
                                        }

                                        // If this is a public upload, get the key details to find the public address
                                        if public {
                                            // Fetch the key details to get the public address
                                            spawn_local({
                                                let ctx = context::context();
                                                let key_name = key_name.clone();
                                                let public_address_clone = public_address.clone();

                                                async move {
                                                    // Wait a moment for the key to be fully registered
                                                    // In a real implementation, we would use a proper delay
                                                    // For now, we'll just continue immediately

                                                    // Invalidate the keys cache to ensure we get fresh data
                                                    ctx.invalidate_caches();

                                                    // Fetch the keys
                                                    let (keys, _) = ctx._list_keys().await;

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

                                        // Show notification
                                        notifications::info("Upload complete!".to_string());
                                    }
                                }
                            },
                            Ok(_) => {
                                log::warn!("Unexpected progress type");
                            },
                            Err(e) => {
                                log::error!("Progress error: {}", e);
                                *error_message.write().unwrap() = Some(e);
                            }
                        }
                    }

                    // If we get here, the progress channel has closed
                    // Make sure the upload is marked as complete
                    if !*upload_complete.read().unwrap() {
                        *is_uploading.write().unwrap() = false;
                        *upload_complete.write().unwrap() = true;

                        // Set elapsed time
                        if let Some(start) = *start_time.read().unwrap() {
                            *elapsed_time.write().unwrap() = start.elapsed().unwrap();
                        }
                    }
                },
                Err(e) => {
                    log::error!("Failed to start upload: {}", e);
                    *error_message.write().unwrap() = Some(e.clone());
                    *is_uploading.write().unwrap() = false;

                    // Show notification
                    notifications::error(format!("Upload failed: {}", e));
                }
            }
        });
    }

    fn reset(&self) {
        // Reset all state for a new upload
        *self.selected_file.write().unwrap() = None;
        *self.file_size.write().unwrap() = None;
        *self.file_data.write().unwrap() = None;
        *self.key_name.write().unwrap() = String::new();
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;
        *self.is_uploading.write().unwrap() = false;
        *self.upload_complete.write().unwrap() = false;
        *self.public_address.write().unwrap() = None;
        *self.error_message.write().unwrap() = None;
        *self.start_time.write().unwrap() = None;
        *self.elapsed_time.write().unwrap() = Duration::from_secs(0);
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
