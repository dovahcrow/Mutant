use std::sync::{Arc, RwLock};

use eframe::egui::{self, RichText};
use mutant_protocol::StorageMode;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;
use web_time::{Duration, SystemTime};

use super::Window;
use super::components::progress::detailed_progress;
use super::components::file_picker::FilePicker;
use super::context;
use super::notifications;
use super::theme::{MutantColors, primary_button, success_button};

#[derive(Clone, Serialize, Deserialize)]
pub enum PutStep {
    FilePicker,
    Configuration,
    Upload,
}

impl Default for PutStep {
    fn default() -> Self {
        PutStep::FilePicker
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PutWindow {
    // Current step in the upload process
    current_step: Arc<RwLock<PutStep>>,
    
    // File picker for selecting files from daemon filesystem
    #[serde(skip)]
    file_picker: Arc<RwLock<Option<FilePicker>>>,
    
    // Selected file path from the file picker
    selected_file_path: Arc<RwLock<Option<String>>>,

    // File selection state (for display)
    selected_file: Arc<RwLock<Option<String>>>,
    file_size: Arc<RwLock<Option<u64>>>,

    // Key name input
    key_name: Arc<RwLock<String>>,

    // Configuration options
    public: Arc<RwLock<bool>>,
    storage_mode: Arc<RwLock<StorageMode>>,
    no_verify: Arc<RwLock<bool>>,

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

    /// Unique identifier for this window instance to avoid widget ID conflicts
    #[serde(skip)]
    window_id: String,
}

impl Default for PutWindow {
    fn default() -> Self {
        Self {
            current_step: Arc::new(RwLock::new(PutStep::FilePicker)),
            file_picker: Arc::new(RwLock::new(None)),
            selected_file_path: Arc::new(RwLock::new(None)),
            selected_file: Arc::new(RwLock::new(None)),
            file_size: Arc::new(RwLock::new(None)),
            key_name: Arc::new(RwLock::new(String::new())),
            public: Arc::new(RwLock::new(false)),
            storage_mode: Arc::new(RwLock::new(StorageMode::Heaviest)),
            no_verify: Arc::new(RwLock::new(false)),

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
            window_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

impl Window for PutWindow {
    fn name(&self) -> String {
        "MutAnt Upload".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        // If we're uploading, check the progress before drawing the form
        if *self.is_uploading.read().unwrap() && !*self.upload_complete.read().unwrap() {
            self.check_progress();
        }

        // Draw the form with the updated progress values
        self.draw_upload_form(ui);

        // Request a repaint to ensure we update frequently
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

impl PutWindow {
    pub fn new() -> Self {
        Self::default()
    }

    // Create a new PutWindow with pre-selected file info (for backward compatibility)
    pub fn new_with_file(filename: String, file_size: u64) -> Self {
        let window = Self::default();

        // Set the selected file info and skip to configuration step
        *window.selected_file.write().unwrap() = Some(filename.clone());
        *window.file_size.write().unwrap() = Some(file_size);
        *window.key_name.write().unwrap() = filename;
        *window.current_step.write().unwrap() = PutStep::Configuration;

        window
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
                        *self.current_step.write().unwrap() = PutStep::Upload;

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

    fn draw_upload_form(&mut self, ui: &mut egui::Ui) {
        let current_step = self.current_step.read().unwrap().clone();
        let is_uploading = *self.is_uploading.read().unwrap();
        let upload_complete = *self.upload_complete.read().unwrap();

        match current_step {
            PutStep::FilePicker => {
                self.draw_file_picker_step(ui);
            }
            PutStep::Configuration => {
                self.draw_configuration_step(ui);
            }
            PutStep::Upload => {
                if upload_complete {
                    self.draw_upload_complete_step(ui);
                } else if is_uploading {
                    self.draw_upload_progress_step(ui);
                } else {
                    // This shouldn't happen, but fallback to configuration
                    *self.current_step.write().unwrap() = PutStep::Configuration;
                }
            }
        }
    }

    fn draw_file_picker_step(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("📁 Select File").size(20.0).color(MutantColors::TEXT_PRIMARY));
        ui.add_space(15.0);

        ui.label(RichText::new("Choose a file from the daemon's filesystem:").color(MutantColors::TEXT_SECONDARY));
        ui.add_space(10.0);

        // Initialize file picker if needed
        if self.file_picker.read().unwrap().is_none() {
            *self.file_picker.write().unwrap() = Some(FilePicker::new().with_files_only(true));
        }

        // Draw the file picker
        if let Some(ref mut picker) = *self.file_picker.write().unwrap() {
            if picker.draw(ui) {
                // File was selected
                if let Some(selected_path) = picker.selected_file() {
                    *self.selected_file_path.write().unwrap() = Some(selected_path.clone());

                    // Extract filename from path for display
                    let filename = std::path::Path::new(&selected_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| selected_path.clone());

                    *self.selected_file.write().unwrap() = Some(filename.clone());
                    *self.key_name.write().unwrap() = filename;

                    // Move to configuration step
                    *self.current_step.write().unwrap() = PutStep::Configuration;
                }
            }
        }

        ui.add_space(20.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                // Reset and close window (this would need to be handled by the window system)
                self.reset();
            }
        });
    }

    fn draw_configuration_step(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("📤 Configure Upload").size(20.0).color(MutantColors::TEXT_PRIMARY));
        ui.add_space(15.0);

        // Show selected file info
        if let Some(filename) = &*self.selected_file.read().unwrap() {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Selected File:").color(MutantColors::TEXT_SECONDARY));
                    ui.label(RichText::new(filename).color(MutantColors::ACCENT_BLUE));

                    if let Some(path) = &*self.selected_file_path.read().unwrap() {
                        ui.label(RichText::new(format!("Path: {}", path)).color(MutantColors::TEXT_MUTED));
                    }
                });
            });
            ui.add_space(10.0);
        }

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

                egui::ComboBox::new(format!("mutant_put_storage_mode_{}", self.window_id), "")
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

        ui.add_space(15.0);

        // Navigation buttons
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                *self.current_step.write().unwrap() = PutStep::FilePicker;
            }

            ui.add_space(10.0);

            let can_upload = !self.key_name.read().unwrap().is_empty() && self.selected_file_path.read().unwrap().is_some();
            if ui.add_enabled(can_upload, primary_button("🚀 Start Upload")).clicked() {
                self.start_path_upload();
            }

            if !can_upload {
                ui.add_space(5.0);
                if self.selected_file_path.read().unwrap().is_none() {
                    ui.label(RichText::new("⚠ No file selected").color(MutantColors::WARNING));
                } else {
                    ui.label(RichText::new("⚠ Please enter a key name").color(MutantColors::WARNING));
                }
            }
        });

        // Show error message if any
        if let Some(error) = &*self.error_message.read().unwrap() {
            ui.add_space(10.0);
            ui.group(|ui| {
                ui.label(RichText::new(format!("❌ Error: {}", error)).color(MutantColors::ERROR));
            });
        }
    }

    fn draw_upload_progress_step(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("🚀 Uploading...").size(20.0).color(MutantColors::TEXT_PRIMARY));
        ui.add_space(15.0);

        // Show file being uploaded
        if let Some(filename) = &*self.selected_file.read().unwrap() {
            ui.label(RichText::new(format!("Uploading: {}", filename)).color(MutantColors::TEXT_SECONDARY));
            ui.add_space(10.0);
        }

        // Show upload progress bars
        let total_chunks = *self.total_chunks.read().unwrap();
        let reservation_progress = *self.reservation_progress.read().unwrap();
        let upload_progress = *self.upload_progress.read().unwrap();
        let confirmation_progress = *self.confirmation_progress.read().unwrap();
        let elapsed = *self.elapsed_time.read().unwrap();
        let elapsed_str = format!("{:.1}s", elapsed.as_secs_f64());

        // Reservation progress
        ui.label("Reserving pads:");
        ui.add(detailed_progress(
            reservation_progress,
            (reservation_progress * total_chunks as f32) as usize,
            total_chunks,
            elapsed_str.clone()
        ));

        ui.add_space(5.0);

        // Upload progress
        ui.label("Uploading pads:");
        ui.add(detailed_progress(
            upload_progress,
            (upload_progress * total_chunks as f32) as usize,
            total_chunks,
            elapsed_str.clone()
        ));

        ui.add_space(5.0);

        // Confirmation progress
        ui.label("Confirming pads:");
        ui.add(detailed_progress(
            confirmation_progress,
            (confirmation_progress * total_chunks as f32) as usize,
            total_chunks,
            elapsed_str
        ));
    }

    fn draw_upload_complete_step(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("✅ Upload Complete!").size(20.0).color(MutantColors::ACCENT_GREEN));
        ui.add_space(15.0);

        // Show completion details
        if let Some(filename) = &*self.selected_file.read().unwrap() {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Successfully uploaded:").color(MutantColors::TEXT_SECONDARY));
                    ui.label(RichText::new(filename).color(MutantColors::ACCENT_BLUE));
                    ui.label(RichText::new(format!("Key: {}", self.key_name.read().unwrap())).color(MutantColors::TEXT_PRIMARY));

                    let elapsed = *self.elapsed_time.read().unwrap();
                    ui.label(RichText::new(format!("Time: {:.1}s", elapsed.as_secs_f64())).color(MutantColors::TEXT_MUTED));
                });
            });
        }

        ui.add_space(10.0);

        // Show public address if available
        if let Some(public_addr) = &*self.public_address.read().unwrap() {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Public Address:").color(MutantColors::TEXT_SECONDARY));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(public_addr).color(MutantColors::ACCENT_ORANGE));
                        if ui.button("📋 Copy").clicked() {
                            ui.ctx().copy_text(public_addr.clone());
                            notifications::info("Public address copied to clipboard!".to_string());
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }

        // Action buttons
        ui.horizontal(|ui| {
            if ui.add(success_button("🔄 Upload Another")).clicked() {
                self.reset();
            }

            ui.add_space(10.0);

            if ui.button("Close").clicked() {
                // This would need to be handled by the window system
                // For now, just reset
                self.reset();
            }
        });
    }

    fn start_path_upload(&self) {
        let file_path = match &*self.selected_file_path.read().unwrap() {
            Some(path) => path.clone(),
            None => {
                *self.error_message.write().unwrap() = Some("No file selected".to_string());
                return;
            }
        };

        let key_name = self.key_name.read().unwrap().clone();
        if key_name.is_empty() {
            *self.error_message.write().unwrap() = Some("Key name cannot be empty".to_string());
            return;
        }

        // Clear any previous error
        *self.error_message.write().unwrap() = None;

        // Set upload state
        *self.is_uploading.write().unwrap() = true;
        *self.upload_complete.write().unwrap() = false;
        *self.start_time.write().unwrap() = Some(SystemTime::now());
        *self.current_step.write().unwrap() = PutStep::Upload;

        // Reset progress
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;

        // Start the upload using file path
        let ctx = context::context();
        let public = *self.public.read().unwrap();
        let storage_mode = self.storage_mode.read().unwrap().clone();
        let no_verify = *self.no_verify.read().unwrap();

        let current_put_id = self.current_put_id.clone();
        let is_uploading = self.is_uploading.clone();
        let error_message = self.error_message.clone();

        // Create progress tracking for this upload
        let (progress_id, progress) = ctx.create_progress(&key_name, &file_path);

        // Store progress references for UI updates
        let reservation_progress = self.reservation_progress.clone();
        let upload_progress = self.upload_progress.clone();
        let confirmation_progress = self.confirmation_progress.clone();
        let upload_complete = self.upload_complete.clone();

        spawn_local(async move {
            log::info!("Starting file path upload: {} -> {}", file_path, key_name);
            match ctx.put_file_path(&key_name, &file_path, public, storage_mode, no_verify).await {
                Ok(put_id) => {
                    log::info!("File path upload started successfully with put_id: {}", put_id);
                    *current_put_id.write().unwrap() = Some(put_id.clone());

                    // Start a progress monitoring task
                    let progress_clone = progress.clone();
                    let is_uploading_clone = is_uploading.clone();
                    let upload_complete_clone = upload_complete.clone();
                    let _error_message_clone = error_message.clone();

                    spawn_local(async move {
                        // Monitor progress updates
                        loop {
                            // WASM-compatible sleep using setTimeout
                            {
                                use wasm_bindgen_futures::JsFuture;
                                use js_sys;
                                let promise = js_sys::Promise::new(&mut |resolve, _| {
                                    web_sys::window()
                                        .unwrap()
                                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 100)
                                        .unwrap();
                                });
                                let _ = JsFuture::from(promise).await;
                            }

                            let progress_data = progress_clone.read().unwrap();
                            let mut has_progress = false;

                            // Update progress bars based on operation data
                            for (operation, value) in &progress_data.operation {
                                has_progress = true;
                                match operation.as_str() {
                                    "reservation" => {
                                        let progress = if value.nb_to_reserve > 0 {
                                            value.nb_reserved as f32 / value.nb_to_reserve as f32
                                        } else {
                                            0.0
                                        };
                                        *reservation_progress.write().unwrap() = progress;
                                    }
                                    "upload" => {
                                        let progress = if value.total_pads > 0 {
                                            value.nb_written as f32 / value.total_pads as f32
                                        } else {
                                            0.0
                                        };
                                        *upload_progress.write().unwrap() = progress;
                                    }
                                    "confirmation" => {
                                        let progress = if value.total_pads > 0 {
                                            value.nb_confirmed as f32 / value.total_pads as f32
                                        } else {
                                            0.0
                                        };
                                        *confirmation_progress.write().unwrap() = progress;
                                    }
                                    _ => {}
                                }
                            }

                            // Check if upload is complete
                            if let Some(confirmation) = progress_data.operation.get("confirmation") {
                                if confirmation.total_pads > 0 && confirmation.nb_confirmed >= confirmation.total_pads {
                                    *upload_complete_clone.write().unwrap() = true;
                                    *is_uploading_clone.write().unwrap() = false;
                                    break;
                                }
                            }

                            // Check if upload is still active
                            if !*is_uploading_clone.read().unwrap() {
                                break;
                            }
                        }
                    });
                }
                Err(e) => {
                    *is_uploading.write().unwrap() = false;
                    *error_message.write().unwrap() = Some(format!("Failed to start upload: {}", e));
                }
            }
        });
    }

    fn reset(&self) {
        *self.current_step.write().unwrap() = PutStep::FilePicker;
        *self.file_picker.write().unwrap() = None;
        *self.selected_file_path.write().unwrap() = None;
        *self.selected_file.write().unwrap() = None;
        *self.file_size.write().unwrap() = None;
        *self.key_name.write().unwrap() = String::new();
        *self.public.write().unwrap() = false;
        *self.storage_mode.write().unwrap() = StorageMode::Heaviest;
        *self.no_verify.write().unwrap() = false;
        *self.is_uploading.write().unwrap() = false;
        *self.upload_complete.write().unwrap() = false;
        *self.public_address.write().unwrap() = None;
        *self.error_message.write().unwrap() = None;
        *self.current_put_id.write().unwrap() = None;
        *self.reservation_progress.write().unwrap() = 0.0;
        *self.upload_progress.write().unwrap() = 0.0;
        *self.confirmation_progress.write().unwrap() = 0.0;
        *self.total_chunks.write().unwrap() = 0;
        *self.chunks_to_reserve.write().unwrap() = 0;
        *self.initial_written_count.write().unwrap() = 0;
        *self.initial_confirmed_count.write().unwrap() = 0;
        *self.start_time.write().unwrap() = None;
        *self.elapsed_time.write().unwrap() = Duration::from_secs(0);
    }
}
