use std::sync::{Arc, RwLock};
use std::collections::HashMap;

use eframe::egui::{self, RichText};
use mutant_protocol::FileSystemEntry;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

use crate::app::context;
use crate::app::theme::MutantColors;

/// Format file size in human-readable format
fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

/// Format Unix timestamp to human-readable date
fn format_modified_time(timestamp: u64) -> String {
    // For web, we'll use a simple format since we can't easily use chrono
    // This is a basic implementation - in a real app you might want to use js_sys
    let now = js_sys::Date::now() / 1000.0; // Current time in seconds
    let diff = now - timestamp as f64;

    if diff < 60.0 {
        "Just now".to_string()
    } else if diff < 3600.0 {
        format!("{:.0}m ago", diff / 60.0)
    } else if diff < 86400.0 {
        format!("{:.0}h ago", diff / 3600.0)
    } else if diff < 2592000.0 {
        format!("{:.0}d ago", diff / 86400.0)
    } else {
        // For older files, show a basic date format
        let date = js_sys::Date::new(&(timestamp as f64 * 1000.0).into());
        format!("{}/{}/{}",
            date.get_month() + 1,
            date.get_date(),
            date.get_full_year()
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FilePicker {
    /// Current directory path
    current_path: Arc<RwLock<String>>,
    /// Directory contents cache
    directory_cache: Arc<RwLock<HashMap<String, Vec<FileSystemEntry>>>>,
    /// Currently selected file path
    selected_file: Arc<RwLock<Option<String>>>,
    /// Loading state for directory operations
    is_loading: Arc<RwLock<bool>>,
    /// Error message if any
    error_message: Arc<RwLock<Option<String>>>,
    /// Expanded directories in tree view
    expanded_dirs: Arc<RwLock<HashMap<String, bool>>>,
    /// Whether to show only files (not directories for selection)
    files_only: bool,
    /// Whether to show hidden files (files starting with '.')
    show_hidden: Arc<RwLock<bool>>,
}

impl Default for FilePicker {
    fn default() -> Self {
        Self::new()
    }
}

impl FilePicker {
    pub fn new() -> Self {
        // Start with the user's home directory or root
        let initial_path = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        
        let picker = Self {
            current_path: Arc::new(RwLock::new(initial_path.clone())),
            directory_cache: Arc::new(RwLock::new(HashMap::new())),
            selected_file: Arc::new(RwLock::new(None)),
            is_loading: Arc::new(RwLock::new(false)),
            error_message: Arc::new(RwLock::new(None)),
            expanded_dirs: Arc::new(RwLock::new(HashMap::new())),
            files_only: true,
            show_hidden: Arc::new(RwLock::new(false)),
        };

        // Load initial directory
        picker.load_directory(&initial_path);
        picker
    }

    pub fn with_files_only(mut self, files_only: bool) -> Self {
        self.files_only = files_only;
        self
    }

    /// Get the currently selected file path
    pub fn selected_file(&self) -> Option<String> {
        self.selected_file.read().unwrap().clone()
    }

    /// Toggle showing hidden files
    pub fn toggle_hidden_files(&self) {
        let mut show_hidden = self.show_hidden.write().unwrap();
        *show_hidden = !*show_hidden;
        // Reload current directory to apply filter
        let current_path = self.current_path.read().unwrap().clone();
        self.load_directory(&current_path);
    }

    /// Check if a file should be shown based on hidden file filter
    fn should_show_entry(&self, entry: &FileSystemEntry) -> bool {
        let show_hidden = *self.show_hidden.read().unwrap();
        if show_hidden {
            true
        } else {
            // Hide files and directories starting with '.'
            !entry.name.starts_with('.')
        }
    }

    /// Load directory contents asynchronously
    fn load_directory(&self, path: &str) {
        let path = path.to_string();
        let current_path = self.current_path.clone();
        let directory_cache = self.directory_cache.clone();
        let is_loading = self.is_loading.clone();
        let error_message = self.error_message.clone();

        *is_loading.write().unwrap() = true;
        *error_message.write().unwrap() = None;

        spawn_local(async move {
            let ctx = context::context();
            
            match ctx.list_directory(&path).await {
                Ok(response) => {
                    // Update cache
                    directory_cache.write().unwrap().insert(path.clone(), response.entries);
                    *current_path.write().unwrap() = path;
                }
                Err(e) => {
                    *error_message.write().unwrap() = Some(format!("Failed to load directory: {}", e));
                }
            }
            
            *is_loading.write().unwrap() = false;
        });
    }

    /// Navigate to a directory
    pub fn navigate_to(&self, path: &str) {
        self.load_directory(path);
    }

    /// Navigate to parent directory
    pub fn navigate_up(&self) {
        let current = self.current_path.read().unwrap().clone();
        if let Some(parent) = std::path::Path::new(&current).parent() {
            let parent_path = parent.to_string_lossy().to_string();
            if !parent_path.is_empty() && parent_path != current {
                self.navigate_to(&parent_path);
            }
        }
    }

    /// Select a file
    pub fn select_file(&self, path: &str) {
        if self.files_only {
            // Only allow file selection, not directories
            spawn_local({
                let path = path.to_string();
                let selected_file = self.selected_file.clone();
                let error_message = self.error_message.clone();
                
                async move {
                    let ctx = context::context();
                    match ctx.get_file_info(&path).await {
                        Ok(info) => {
                            if info.exists && !info.is_directory {
                                *selected_file.write().unwrap() = Some(path);
                                *error_message.write().unwrap() = None;
                            } else if info.is_directory {
                                *error_message.write().unwrap() = Some("Please select a file, not a directory".to_string());
                            } else {
                                *error_message.write().unwrap() = Some("File does not exist".to_string());
                            }
                        }
                        Err(e) => {
                            *error_message.write().unwrap() = Some(format!("Failed to get file info: {}", e));
                        }
                    }
                }
            });
        } else {
            *self.selected_file.write().unwrap() = Some(path.to_string());
        }
    }

    /// Draw the file picker UI
    pub fn draw(&mut self, ui: &mut egui::Ui) -> bool {
        let mut file_selected = false;

        ui.vertical(|ui| {
            // Header with current path and navigation
            ui.horizontal(|ui| {
                if ui.button("⬆ Up").clicked() {
                    self.navigate_up();
                }

                ui.separator();

                let current_path = self.current_path.read().unwrap().clone();
                ui.label(RichText::new(format!("📁 {}", current_path)).color(MutantColors::TEXT_PRIMARY));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Hidden files toggle
                    let show_hidden = *self.show_hidden.read().unwrap();
                    let toggle_text = if show_hidden { "🙈 Hide Hidden" } else { "👁 Show Hidden" };
                    if ui.button(toggle_text).clicked() {
                        self.toggle_hidden_files();
                    }
                });
            });

            ui.separator();

            // Error message
            if let Some(error) = &*self.error_message.read().unwrap() {
                ui.colored_label(MutantColors::ERROR, format!("❌ {}", error));
                ui.separator();
            }

            // Loading indicator
            if *self.is_loading.read().unwrap() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading...");
                });
                return;
            }

            // Directory contents
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let current_path = self.current_path.read().unwrap().clone();
                    let cache = self.directory_cache.read().unwrap();
                    
                    if let Some(entries) = cache.get(&current_path) {
                        // Filter entries based on hidden file setting
                        let filtered_entries: Vec<_> = entries.iter()
                            .filter(|entry| self.should_show_entry(entry))
                            .collect();

                        // Sort entries: directories first, then files, both alphabetically
                        let mut sorted_entries = filtered_entries;
                        sorted_entries.sort_by(|a, b| {
                            match (a.is_directory, b.is_directory) {
                                (true, false) => std::cmp::Ordering::Less,
                                (false, true) => std::cmp::Ordering::Greater,
                                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                            }
                        });

                        for entry in sorted_entries {
                            self.draw_entry(ui, entry, &mut file_selected);
                        }
                    } else {
                        ui.label("No directory contents loaded");
                    }
                });

            ui.separator();

            // Selected file display
            if let Some(selected) = &*self.selected_file.read().unwrap() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✅ Selected:").color(MutantColors::ACCENT_GREEN));
                        ui.label(RichText::new(selected).color(MutantColors::TEXT_PRIMARY));
                    });
                });
                ui.add_space(10.0);

                // Add a "Next" button for better UX
                ui.horizontal(|ui| {
                    if ui.button("➡ Next").clicked() {
                        file_selected = true;
                    }

                    ui.add_space(10.0);

                    if ui.button("Clear Selection").clicked() {
                        *self.selected_file.write().unwrap() = None;
                    }
                });
            }
        });

        file_selected
    }

    /// Draw a single file or directory entry with stats
    fn draw_entry(&self, ui: &mut egui::Ui, entry: &FileSystemEntry, file_selected: &mut bool) {
        // Check if this entry is currently selected
        let is_selected = if let Some(selected_path) = &*self.selected_file.read().unwrap() {
            selected_path == &entry.path
        } else {
            false
        };

        // Use a frame to highlight selected files
        let frame = if is_selected {
            egui::Frame::default()
                .fill(MutantColors::ACCENT_ORANGE.gamma_multiply(0.2))
                .stroke(egui::Stroke::new(1.0, MutantColors::ACCENT_ORANGE))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(4))
        } else {
            egui::Frame::default()
                .inner_margin(egui::Margin::same(4))
        };

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Icon and name
                let icon = if entry.is_directory { "📁" } else { "📄" };
                let name_color = if entry.is_directory {
                    MutantColors::ACCENT_BLUE
                } else if is_selected {
                    MutantColors::ACCENT_ORANGE
                } else {
                    MutantColors::TEXT_PRIMARY
                };

                // Main button with file/directory name
                let button_response = ui.button(
                    RichText::new(format!("{} {}", icon, entry.name))
                        .color(name_color)
                ).on_hover_text(&entry.path);

                if button_response.clicked() {
                    if entry.is_directory {
                        self.navigate_to(&entry.path);
                    } else {
                        self.select_file(&entry.path);
                        *file_selected = true;
                    }
                }

            // Add spacing before stats
            ui.add_space(10.0);

            // File stats (size and modified time)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Modified time
                if let Some(modified) = entry.modified {
                    ui.label(
                        RichText::new(format_modified_time(modified))
                            .color(MutantColors::TEXT_MUTED)
                            .size(11.0)
                    );
                    ui.add_space(10.0);
                }

                // File size (only for files)
                if !entry.is_directory {
                    if let Some(size) = entry.size {
                        ui.label(
                            RichText::new(format_file_size(size))
                                .color(MutantColors::TEXT_MUTED)
                                .size(11.0)
                        );
                    } else {
                        ui.label(
                            RichText::new("--")
                                .color(MutantColors::TEXT_MUTED)
                                .size(11.0)
                        );
                    }
                } else {
                    // For directories, show folder indicator
                    ui.label(
                        RichText::new("<DIR>")
                            .color(MutantColors::ACCENT_BLUE)
                            .size(11.0)
                    );
                }
            });
            });
        });
    }
}


