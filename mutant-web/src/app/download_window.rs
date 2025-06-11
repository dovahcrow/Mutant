use eframe::egui;
use mutant_protocol::{KeyDetails, TaskProgress, GetEvent};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use web_time::{Duration, SystemTime};

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

    // Progress tracking fields
    /// Total number of chunks to download
    #[serde(skip)]
    total_chunks: Arc<RwLock<usize>>,
    /// Number of chunks fetched so far
    #[serde(skip)]
    fetched_chunks: Arc<RwLock<usize>>,
    /// Download progress (0.0 to 1.0)
    #[serde(skip)]
    progress: Arc<RwLock<f32>>,
    /// Start time for elapsed time calculation
    #[serde(skip)]
    start_time: Arc<RwLock<Option<SystemTime>>>,
    /// Elapsed time
    #[serde(skip)]
    elapsed_time: Arc<RwLock<Duration>>,
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
            total_chunks: Arc::new(RwLock::new(0)),
            fetched_chunks: Arc::new(RwLock::new(0)),
            progress: Arc::new(RwLock::new(0.0)),
            start_time: Arc::new(RwLock::new(None)),
            elapsed_time: Arc::new(RwLock::new(Duration::from_secs(0))),
        }
    }

    /// Draw the download window UI
    pub fn draw(&mut self, ui: &mut egui::Ui) -> DownloadWindowResponse {
        let mut response = DownloadWindowResponse::None;

        // Check if download is complete (progress reached 1.0)
        if self.downloading && !self.download_completed {
            let current_progress = *self.progress.read().unwrap();
            if current_progress >= 1.0 {
                self.downloading = false;
                self.download_completed = true;
            }
        }

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

            // Title - centered
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("📥 Download File")
                        .size(18.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );
            });

            ui.add_space(10.0);

            // File information - centered
            ui.vertical_centered(|ui| {
                // Create single centered labels instead of horizontal layouts
                ui.label(
                    egui::RichText::new(format!("File: {}", &self.file_details.key))
                        .size(14.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                        .monospace()
                );
                ui.label(
                    egui::RichText::new(format!("Size: {}", &crate::app::fs::tree::humanize_size(self.file_details.total_size)))
                        .size(14.0)
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );
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

        // Request repaint for progress updates
        if self.downloading {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        response
    }

    /// Draw the directory picker and filename editing interface using side-by-side layout like put window
    fn draw_file_picker_ui(&mut self, ui: &mut egui::Ui, response: &mut DownloadWindowResponse) {
        // Use the full available space without any margins, similar to put window
        let available_rect = ui.available_rect_before_wrap();
        let content_height = available_rect.height();
        let content_width = available_rect.width();

        // Force the UI to expand to the full available rectangle
        ui.expand_to_include_rect(available_rect);

        // Use horizontal layout that fills the entire space (50/50 split like put window)
        ui.horizontal(|ui| {
            // Left side: Directory picker (50% width, full height)
            let left_width = content_width * 0.5;

            ui.allocate_ui_with_layout(
                egui::Vec2::new(left_width, content_height - 15.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    // Initialize directory picker if needed
                    if self.directory_picker.is_none() {
                        self.directory_picker = Some(
                            crate::app::components::file_picker::FilePicker::new()
                                .with_files_only(false) // Directory selection mode
                        );
                    }

                    // Draw the directory picker using ALL available space
                    if let Some(ref mut picker) = self.directory_picker {
                        picker.draw(ui);
                    }
                }
            );

            // Right side: Download configuration (50% width, full height) - NO spacing between panels
            let right_width = content_width * 0.5;

            ui.allocate_ui_with_layout(
                egui::Vec2::new(right_width, content_height),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    // Simple centering with equal top/bottom margins
                    let available_height = ui.available_height();
                    let margin = available_height * 0.1; // 10% margin top and bottom

                    ui.add_space(margin);

                    ui.vertical_centered(|ui| {
                        // Constrain the content width for better readability
                        let max_content_width = (right_width * 0.8).min(350.0);

                        ui.allocate_ui_with_layout(
                            egui::Vec2::new(max_content_width, available_height - (margin * 2.0)),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                // Header for download configuration - centered
                                ui.vertical_centered(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("📥").size(20.0).color(super::theme::MutantColors::ACCENT_BLUE));
                                        ui.add_space(8.0);
                                        ui.heading(egui::RichText::new("Download Configuration").size(18.0).color(super::theme::MutantColors::TEXT_PRIMARY));
                                    });
                                });
                                ui.add_space(12.0);

                                // Show file info with enhanced styling - centered
                                ui.vertical_centered(|ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("📄").size(14.0).color(super::theme::MutantColors::ACCENT_ORANGE));
                                                ui.label(egui::RichText::new("File to Download").size(13.0).color(super::theme::MutantColors::TEXT_SECONDARY).strong());
                                            });
                                            ui.add_space(4.0);
                                            ui.label(egui::RichText::new(&self.file_details.key).size(14.0).color(super::theme::MutantColors::ACCENT_ORANGE).strong());
                                            ui.label(egui::RichText::new(&crate::app::fs::tree::humanize_size(self.file_details.total_size)).size(11.0).color(super::theme::MutantColors::TEXT_MUTED));
                                        });
                                    });
                                });

                                ui.add_space(12.0);

                                // Selected directory info
                                let selected_directory = self.directory_picker
                                    .as_ref()
                                    .and_then(|picker| picker.selected_file_full_path());

                                ui.vertical_centered(|ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("📂").size(14.0).color(super::theme::MutantColors::ACCENT_BLUE));
                                                ui.label(egui::RichText::new("Destination Folder").size(13.0).color(super::theme::MutantColors::TEXT_SECONDARY).strong());
                                            });
                                            ui.add_space(4.0);
                                            if let Some(directory) = &selected_directory {
                                                ui.label(egui::RichText::new(directory).size(12.0).color(super::theme::MutantColors::ACCENT_BLUE).monospace());
                                            } else {
                                                ui.label(egui::RichText::new("← Select a folder from the left panel").size(12.0).color(super::theme::MutantColors::TEXT_MUTED).italics());
                                            }
                                        });
                                    });
                                });

                                ui.add_space(12.0);

                                // Filename input with enhanced styling - centered
                                ui.vertical_centered(|ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("✏️").size(14.0).color(super::theme::MutantColors::ACCENT_GREEN));
                                                ui.label(egui::RichText::new("Filename").size(13.0).color(super::theme::MutantColors::TEXT_PRIMARY).strong());
                                            });
                                            ui.add_space(6.0);
                                            let text_edit = egui::TextEdit::singleline(&mut self.filename)
                                                .desired_width(ui.available_width() - 10.0)
                                                .font(egui::TextStyle::Monospace);
                                            ui.add(text_edit);
                                        });
                                    });
                                });

                                ui.add_space(20.0);

                                // Action buttons with enhanced styling - centered
                                let download_enabled = selected_directory.is_some() && !self.filename.trim().is_empty();
                                let selected_directory_clone = selected_directory.clone(); // Clone for use in closure

                                ui.vertical_centered(|ui| {
                                    ui.horizontal(|ui| {
                                        // Cancel button
                                        let cancel_button = egui::Button::new(egui::RichText::new("Cancel").color(super::theme::MutantColors::TEXT_SECONDARY))
                                            .fill(super::theme::MutantColors::BACKGROUND_MEDIUM)
                                            .stroke(egui::Stroke::new(1.0, super::theme::MutantColors::BORDER_LIGHT))
                                            .min_size([80.0, 32.0].into());

                                        if ui.add(cancel_button).clicked() {
                                            *response = DownloadWindowResponse::Cancel;
                                        }

                                        ui.add_space(10.0);

                                        // Download button
                                        let download_button = if download_enabled {
                                            egui::Button::new(egui::RichText::new("📥 Download").size(14.0).strong().color(super::theme::MutantColors::TEXT_PRIMARY))
                                                .fill(super::theme::MutantColors::ACCENT_GREEN)
                                                .stroke(egui::Stroke::new(2.0, super::theme::MutantColors::ACCENT_GREEN))
                                                .min_size([120.0, 32.0].into())
                                        } else {
                                            egui::Button::new(egui::RichText::new("📥 Download").size(14.0).color(super::theme::MutantColors::TEXT_MUTED))
                                                .fill(super::theme::MutantColors::SURFACE)
                                                .stroke(egui::Stroke::new(1.0, super::theme::MutantColors::BORDER_MEDIUM))
                                                .min_size([120.0, 32.0].into())
                                        };

                                        if ui.add_enabled(download_enabled, download_button).clicked() {
                                            if let Some(directory) = selected_directory_clone {
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

                                if !download_enabled {
                                    ui.add_space(8.0);
                                    ui.vertical_centered(|ui| {
                                        if selected_directory.is_none() {
                                            ui.label(egui::RichText::new("⚠ Please select a destination folder").size(12.0).color(super::theme::MutantColors::WARNING));
                                        } else {
                                            ui.label(egui::RichText::new("⚠ Please enter a filename").size(12.0).color(super::theme::MutantColors::WARNING));
                                        }
                                    });
                                }
                            }
                        );
                    });
                }
            );
        });
    }



    /// Draw downloading UI with progress
    fn draw_downloading_ui(&self, ui: &mut egui::Ui) {
        // Use the full available space without any margins, similar to put window
        let available_rect = ui.available_rect_before_wrap();
        ui.expand_to_include_rect(available_rect);

        let available_width = available_rect.width();
        let available_height = available_rect.height();

        // Use the full available space without any inner margins
        ui.allocate_ui_with_layout(
            egui::Vec2::new(available_width, available_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                // Add vertical spacing to center content
                let content_height = 250.0; // Approximate height of our content
                let top_spacing = (available_height - content_height) / 2.0;
                if top_spacing > 0.0 {
                    ui.add_space(top_spacing);
                }

                // Constrain content width for better readability
                let max_content_width = (available_width * 0.8).min(500.0);

                ui.allocate_ui_with_layout(
                    egui::Vec2::new(max_content_width, content_height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        // Header
                        ui.vertical_centered(|ui| {
                            ui.heading(
                                egui::RichText::new("📥 Downloading...")
                                    .size(20.0)
                                    .color(super::theme::MutantColors::TEXT_PRIMARY),
                            );
                        });
                        ui.add_space(15.0);

                        // Show file being downloaded
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Downloading: {}", self.filename))
                                    .color(super::theme::MutantColors::TEXT_SECONDARY),
                            );
                        });
                        ui.add_space(10.0);

                        // Progress information
                        let total_chunks = *self.total_chunks.read().unwrap();
                        let fetched_chunks = *self.fetched_chunks.read().unwrap();
                        let progress = *self.progress.read().unwrap();
                        let elapsed = *self.elapsed_time.read().unwrap();
                        let elapsed_str = format!("{:.1}s", elapsed.as_secs_f64());

                        // Progress bar with full width
                        ui.vertical(|ui| {
                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("📦 Fetching chunks...")
                                            .color(super::theme::MutantColors::ACCENT_BLUE)
                                            .strong()
                                    );

                                    ui.add_space(5.0);

                                    // Progress bar
                                    ui.add_sized(
                                        [max_content_width, 20.0],
                                        super::components::progress::detailed_progress(
                                            progress,
                                            fetched_chunks,
                                            total_chunks,
                                            elapsed_str,
                                        ),
                                    );

                                    ui.add_space(5.0);

                                    // Additional info
                                    if total_chunks > 0 {
                                        ui.label(
                                            egui::RichText::new(format!("Progress: {}/{} chunks ({:.1}%)",
                                                fetched_chunks, total_chunks, progress * 100.0))
                                                .size(12.0)
                                                .color(super::theme::MutantColors::TEXT_MUTED)
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("Initializing download...")
                                                .size(12.0)
                                                .color(super::theme::MutantColors::TEXT_MUTED)
                                        );
                                    }
                                });
                            });
                        });
                    },
                );
            },
        );
    }

    /// Draw completion UI
    fn draw_completion_ui(&self, ui: &mut egui::Ui, _response: &mut DownloadWindowResponse) {
        // Use the full available space without any margins, similar to downloading UI
        let available_rect = ui.available_rect_before_wrap();
        ui.expand_to_include_rect(available_rect);

        let available_width = available_rect.width();
        let available_height = available_rect.height();

        // Use the full available space without any inner margins
        ui.allocate_ui_with_layout(
            egui::Vec2::new(available_width, available_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                // Add vertical spacing to center content
                let content_height = 200.0; // Approximate height of our content
                let top_spacing = (available_height - content_height) / 2.0;
                if top_spacing > 0.0 {
                    ui.add_space(top_spacing);
                }

                // Constrain content width for better readability
                let max_content_width = (available_width * 0.8).min(400.0);

                ui.allocate_ui_with_layout(
                    egui::Vec2::new(max_content_width, content_height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.vertical_centered(|ui| {
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
                        });
                    },
                );
            },
        );
    }

    /// Start the download process with progress tracking
    pub fn start_download(&mut self, destination_path: String) {
        self.downloading = true;
        self.download_completed = false;
        self.download_error = None;

        // Reset progress tracking
        *self.total_chunks.write().unwrap() = 0;
        *self.fetched_chunks.write().unwrap() = 0;
        *self.progress.write().unwrap() = 0.0;
        *self.start_time.write().unwrap() = Some(SystemTime::now());
        *self.elapsed_time.write().unwrap() = Duration::from_secs(0);

        let file_details = self.file_details.clone();
        let ctx = crate::app::context::context();

        // Clone progress tracking references for the async task
        let total_chunks = self.total_chunks.clone();
        let fetched_chunks = self.fetched_chunks.clone();
        let progress = self.progress.clone();
        let start_time = self.start_time.clone();
        let elapsed_time = self.elapsed_time.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match ctx.download_to_path_with_progress(&file_details.key, &destination_path, file_details.is_public).await {
                Ok(mut progress_rx) => {
                    log::info!("Download started successfully to: {}", destination_path);

                    // Listen for progress updates
                    while let Some(progress_result) = progress_rx.recv().await {
                        match progress_result {
                            Ok(TaskProgress::Get(get_event)) => {
                                match get_event {
                                    GetEvent::Starting { total_chunks: total } => {
                                        log::info!("Download starting: {} total chunks", total);
                                        *total_chunks.write().unwrap() = total;
                                        *fetched_chunks.write().unwrap() = 0;
                                        *progress.write().unwrap() = 0.0;
                                    }
                                    GetEvent::PadFetched => {
                                        let mut fetched = fetched_chunks.write().unwrap();
                                        *fetched += 1;
                                        let total = *total_chunks.read().unwrap();
                                        if total > 0 {
                                            *progress.write().unwrap() = *fetched as f32 / total as f32;
                                        }

                                        // Update elapsed time
                                        if let Some(start) = *start_time.read().unwrap() {
                                            if let Ok(elapsed) = SystemTime::now().duration_since(start) {
                                                *elapsed_time.write().unwrap() = elapsed;
                                            }
                                        }
                                    }
                                    GetEvent::Complete => {
                                        log::info!("Download completed successfully");
                                        *progress.write().unwrap() = 1.0;
                                        // Note: We can't directly update the window state here due to ownership
                                        // The UI will detect completion when progress reaches 1.0
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            Ok(_) => {
                                // Other task progress types (not Get events)
                            }
                            Err(e) => {
                                log::error!("Progress stream error: {}", e);
                                // Note: We can't directly update error state here due to ownership
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Download failed: {}", e);
                    // Note: We can't directly update error state here due to ownership
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
}
