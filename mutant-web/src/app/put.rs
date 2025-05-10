use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, RichText};
use mutant_protocol::StorageMode;
use serde::{Deserialize, Serialize};
// use std::time::{Instant, Duration};
use web_time::{Duration, SystemTime};
use wasm_bindgen_futures::spawn_local;

use super::Window;
use super::components::progress::progress;
use super::notifications;

#[derive(Clone, Serialize, Deserialize)]
pub struct PutWindow {
    // File selection state
    selected_file: Arc<RwLock<Option<String>>>,
    file_size: Arc<RwLock<Option<u64>>>,
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

                    egui::ComboBox::from_id_source("storage_mode")
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

            // Reservation progress bar
            ui.label("Reserving pads:");
            let reservation_progress = *self.reservation_progress.read().unwrap();
            ui.add(progress(reservation_progress, elapsed_str.clone()));

            ui.add_space(5.0);

            // Upload progress bar
            ui.label("Uploading pads:");
            let upload_progress = *self.upload_progress.read().unwrap();
            ui.add(progress(upload_progress, elapsed_str.clone()));

            ui.add_space(5.0);

            // Confirmation progress bar
            ui.label("Confirming pads:");
            let confirmation_progress = *self.confirmation_progress.read().unwrap();
            ui.add(progress(confirmation_progress, elapsed_str));

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
        // This function will be implemented to handle file selection using web-sys
        log::info!("File selection requested");

        // For now, we'll just simulate file selection
        // In a real implementation, we would use web-sys to create a file input element
        let selected_file = self.selected_file.clone();
        let file_size = self.file_size.clone();
        let file_data = self.file_data.clone();

        // Simulate file selection
        *selected_file.write().unwrap() = Some("test_file.txt".to_string());
        *file_size.write().unwrap() = Some(1024);
        *file_data.write().unwrap() = Some(vec![0; 1024]);

        // Show notification
        notifications::info("File selected: test_file.txt".to_string());
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

        // Clone Arc references for the async task
        let reservation_progress = self.reservation_progress.clone();
        let upload_progress = self.upload_progress.clone();
        let confirmation_progress = self.confirmation_progress.clone();
        let is_uploading = self.is_uploading.clone();
        let upload_complete = self.upload_complete.clone();
        let public_address = self.public_address.clone();
        let elapsed_time = self.elapsed_time.clone();
        let start_time = self.start_time.clone();

        // Simulate progress updates
        // In a real implementation, we would use client_manager to perform the actual upload

        // Simulate reservation progress
        *reservation_progress.write().unwrap() = 0.3;

        // After a short delay, simulate upload progress
        *reservation_progress.write().unwrap() = 1.0;
        *upload_progress.write().unwrap() = 0.5;

        // After another delay, simulate confirmation progress
        *upload_progress.write().unwrap() = 1.0;
        *confirmation_progress.write().unwrap() = 0.7;

        // Complete the upload
        *confirmation_progress.write().unwrap() = 1.0;
        *is_uploading.write().unwrap() = false;
        *upload_complete.write().unwrap() = true;

        // Set elapsed time
        if let Some(start) = *start_time.read().unwrap() {
            *elapsed_time.write().unwrap() = start.elapsed().unwrap();
        }

        // Set public address if public
        if public {
            *public_address.write().unwrap() = Some("public_address_placeholder".to_string());
        }

        // Show notification
        notifications::info("Upload complete!".to_string());
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
