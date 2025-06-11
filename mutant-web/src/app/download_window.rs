use eframe::egui;
use mutant_protocol::KeyDetails;
use serde::{Deserialize, Serialize};

/// Download window with directory picker and filename editing functionality
#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadWindow {
    /// The file details to download
    pub file_details: KeyDetails,
    /// Directory picker for selecting destination directory
    #[serde(skip)]
    directory_picker: Option<crate::app::components::file_picker::FilePicker>,
    /// Editable filename for the download
    filename: String,
    /// Download in progress
    #[serde(skip)]
    downloading: bool,
    /// Download completed successfully
    #[serde(skip)]
    download_completed: bool,
    /// Download error message
    #[serde(skip)]
    download_error: Option<String>,
}

impl DownloadWindow {
    pub fn new(file_details: KeyDetails) -> Self {
        // Extract filename from the key for pre-filling
        let filename = std::path::Path::new(&file_details.key)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| file_details.key.clone());

        Self {
            file_details,
            directory_picker: None, // Will be initialized on first draw
            filename,
            downloading: false,
            download_completed: false,
            download_error: None,
        }
    }

    /// Draw the download window UI
    pub fn draw(&mut self, ui: &mut egui::Ui) -> DownloadWindowResponse {
        let mut response = DownloadWindowResponse::None;

        // Initialize directory picker on first draw
        if self.directory_picker.is_none() {
            self.directory_picker = Some(
                crate::app::components::file_picker::FilePicker::new()
                    .with_files_only(false) // Allow directory selection
            );
        }

        // Window title and file info
        ui.vertical(|ui| {
            ui.add_space(10.0);

            // Title
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("📥 Download File")
                        .size(18.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );
            });

            ui.add_space(10.0);

            // File information
            egui::Frame::new()
                .fill(super::theme::MutantColors::BACKGROUND_MEDIUM)
                .corner_radius(6.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("File:")
                                .size(14.0)
                                .color(super::theme::MutantColors::TEXT_SECONDARY)
                        );
                        ui.label(
                            egui::RichText::new(&self.file_details.key)
                                .size(14.0)
                                .color(super::theme::MutantColors::TEXT_PRIMARY)
                                .monospace()
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Size:")
                                .size(14.0)
                                .color(super::theme::MutantColors::TEXT_SECONDARY)
                        );
                        ui.label(
                            egui::RichText::new(&crate::app::fs::tree::humanize_size(self.file_details.total_size))
                                .size(14.0)
                                .color(super::theme::MutantColors::TEXT_PRIMARY)
                        );
                    });
                });

            ui.add_space(15.0);

            // Show different UI based on state
            if self.download_completed {
                self.draw_completion_ui(ui, &mut response);
            } else if self.downloading {
                self.draw_downloading_ui(ui);
            } else {
                self.draw_file_picker_ui(ui, &mut response);
            }
        });

        response
    }

    /// Draw the directory picker and filename editing interface
    fn draw_file_picker_ui(&mut self, ui: &mut egui::Ui, response: &mut DownloadWindowResponse) {
        // Split the UI into two sections: directory picker and filename editor
        ui.vertical(|ui| {
            // Directory selection section
            ui.label(
                egui::RichText::new("1. Choose destination folder:")
                    .size(14.0)
                    .color(super::theme::MutantColors::TEXT_PRIMARY)
                    .strong()
            );

            ui.add_space(4.0);

            // Add helpful instruction
            ui.label(
                egui::RichText::new("Click on a folder to select it. Double-click to expand/collapse.")
                    .size(11.0)
                    .color(super::theme::MutantColors::TEXT_MUTED)
                    .italics()
            );

            ui.add_space(8.0);

            // Directory picker (takes most of the space)
            let available_height = ui.available_height();
            let picker_height = (available_height * 0.7).max(200.0); // 70% of available height, minimum 200px

            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width(), picker_height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    if let Some(ref mut directory_picker) = self.directory_picker {
                        directory_picker.draw(ui);
                    }
                }
            );

            ui.add_space(15.0);

            // Filename editing section
            ui.label(
                egui::RichText::new("2. Edit filename:")
                    .size(14.0)
                    .color(super::theme::MutantColors::TEXT_PRIMARY)
                    .strong()
            );

            ui.add_space(8.0);

            // Filename input field
            egui::Frame::new()
                .fill(super::theme::MutantColors::BACKGROUND_MEDIUM)
                .corner_radius(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Filename:")
                                .size(12.0)
                                .color(super::theme::MutantColors::TEXT_SECONDARY)
                        );

                        let text_edit = egui::TextEdit::singleline(&mut self.filename)
                            .desired_width(ui.available_width() - 10.0)
                            .font(egui::TextStyle::Monospace);

                        ui.add(text_edit);
                    });
                });

            ui.add_space(15.0);

            // Action buttons
            ui.horizontal(|ui| {
                // Cancel button
                if ui.add_sized(
                    [100.0, 32.0],
                    egui::Button::new("Cancel")
                        .fill(super::theme::MutantColors::BACKGROUND_MEDIUM)
                        .stroke(egui::Stroke::new(1.0, super::theme::MutantColors::BORDER_LIGHT))
                ).clicked() {
                    *response = DownloadWindowResponse::Cancel;
                }

                ui.add_space(10.0);

                // Download button
                let selected_directory = self.directory_picker
                    .as_ref()
                    .and_then(|picker| picker.selected_file_full_path());

                let download_enabled = selected_directory.is_some() && !self.filename.trim().is_empty();

                let download_button = egui::Button::new("Download")
                    .fill(if download_enabled {
                        super::theme::MutantColors::ACCENT_GREEN
                    } else {
                        super::theme::MutantColors::BACKGROUND_MEDIUM
                    })
                    .stroke(egui::Stroke::new(1.0, if download_enabled {
                        super::theme::MutantColors::ACCENT_GREEN
                    } else {
                        super::theme::MutantColors::BORDER_LIGHT
                    }));

                if ui.add_sized([120.0, 32.0], download_button).clicked() && download_enabled {
                    if let Some(directory) = selected_directory {
                        // Construct the full download path
                        let full_path = if directory.ends_with('/') {
                            format!("{}{}", directory, self.filename.trim())
                        } else {
                            format!("{}/{}", directory, self.filename.trim())
                        };
                        *response = DownloadWindowResponse::StartDownload(full_path);
                    }
                }
            });
        });
    }



    /// Draw downloading UI
    fn draw_downloading_ui(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Downloading file...")
                        .size(16.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );
            });

            ui.add_space(10.0);

            ui.label(
                egui::RichText::new("Download in progress...")
                    .size(12.0)
                    .color(super::theme::MutantColors::TEXT_MUTED)
            );

            ui.add_space(20.0);
        });
    }

    /// Draw completion UI
    fn draw_completion_ui(&self, ui: &mut egui::Ui, response: &mut DownloadWindowResponse) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);

            if let Some(error) = &self.download_error {
                // Error state
                ui.label(
                    egui::RichText::new("❌ Download Failed")
                        .size(18.0)
                        .color(super::theme::MutantColors::ERROR)
                );

                ui.add_space(10.0);

                ui.label(
                    egui::RichText::new(error)
                        .size(14.0)
                        .color(super::theme::MutantColors::ERROR)
                );
            } else {
                // Success state
                ui.label(
                    egui::RichText::new("✅ Download Complete")
                        .size(18.0)
                        .color(super::theme::MutantColors::ACCENT_GREEN)
                );

                ui.add_space(10.0);

                ui.label(
                    egui::RichText::new("File has been downloaded successfully!")
                        .size(14.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );
            }

            ui.add_space(20.0);

            // Close button
            if ui.add_sized(
                [100.0, 32.0],
                egui::Button::new("Close")
                    .fill(super::theme::MutantColors::ACCENT_BLUE)
                    .stroke(egui::Stroke::new(1.0, super::theme::MutantColors::ACCENT_BLUE))
            ).clicked() {
                *response = DownloadWindowResponse::Close;
            }
        });
    }

    /// Start the download process
    pub fn start_download(&mut self, destination_path: String) {
        self.downloading = true;

        let file_details = self.file_details.clone();
        let ctx = crate::app::context::context();

        wasm_bindgen_futures::spawn_local(async move {
            match ctx.download_to_path(&file_details.key, &destination_path, file_details.is_public).await {
                Ok(_) => {
                    log::info!("Download completed successfully to: {}", destination_path);
                    // TODO: Update completion state - this would need proper state management
                }
                Err(e) => {
                    log::error!("Download failed: {}", e);
                    // TODO: Update error state - this would need proper state management
                }
            }
        });
    }
}

impl crate::app::Window for DownloadWindow {
    fn name(&self) -> String {
        format!("Download: {}", self.filename)
    }

    fn draw(&mut self, ui: &mut eframe::egui::Ui) {
        let _response = self.draw(ui);
        // Note: We ignore the response here since it's handled in the internal tab system
    }
}

/// Response from the download window
#[derive(Debug, Clone)]
pub enum DownloadWindowResponse {
    None,
    Cancel,
    StartDownload(String),
    Close,
}
