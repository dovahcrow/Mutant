use std::sync::{Arc, RwLock};
use std::collections::BTreeMap;

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

/// A node in the file picker tree
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilePickerNode {
    /// Name of this node (file or directory name)
    pub name: String,
    /// Full path to this node
    pub path: String,
    /// Whether this is a directory
    pub is_directory: bool,
    /// File size (only for files)
    pub size: Option<u64>,
    /// Modified timestamp
    pub modified: Option<u64>,
    /// Child nodes (only for directories)
    pub children: BTreeMap<String, FilePickerNode>,
    /// Whether this directory is expanded
    pub expanded: bool,
    /// Whether this directory has been loaded
    pub loaded: bool,
}

impl FilePickerNode {
    /// Create a new directory node
    pub fn new_dir(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_directory: true,
            size: None,
            modified: None,
            children: BTreeMap::new(),
            expanded: false,
            loaded: false,
        }
    }

    /// Create a new file node
    pub fn new_file(entry: &FileSystemEntry) -> Self {
        Self {
            name: entry.name.clone(),
            path: entry.path.clone(),
            is_directory: false,
            size: entry.size,
            modified: entry.modified,
            children: BTreeMap::new(),
            expanded: false,
            loaded: true, // Files are always "loaded"
        }
    }

    /// Insert entries from a directory listing into this node
    pub fn insert_entries(&mut self, entries: &[FileSystemEntry]) {
        self.children.clear();
        self.loaded = true;

        for entry in entries {
            if entry.is_directory {
                let child = FilePickerNode::new_dir(&entry.name, &entry.path);
                self.children.insert(entry.name.clone(), child);
            } else {
                let child = FilePickerNode::new_file(entry);
                self.children.insert(entry.name.clone(), child);
            }
        }
    }

    /// Find a node by path
    pub fn find_node_mut(&mut self, path: &str) -> Option<&mut FilePickerNode> {
        if self.path == path {
            return Some(self);
        }

        for child in self.children.values_mut() {
            if let Some(node) = child.find_node_mut(path) {
                return Some(node);
            }
        }

        None
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FilePicker {
    /// Root node of the file tree
    root: Arc<RwLock<FilePickerNode>>,
    /// Currently selected file path
    selected_file: Arc<RwLock<Option<String>>>,
    /// Loading state for directory operations
    is_loading: Arc<RwLock<bool>>,
    /// Error message if any
    error_message: Arc<RwLock<Option<String>>>,
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

        // Create root node
        let root = FilePickerNode::new_dir("", &initial_path);

        let picker = Self {
            root: Arc::new(RwLock::new(root)),
            selected_file: Arc::new(RwLock::new(None)),
            is_loading: Arc::new(RwLock::new(false)),
            error_message: Arc::new(RwLock::new(None)),
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
        // TODO: Refresh the tree view to apply filter
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
        let root = self.root.clone();
        let is_loading = self.is_loading.clone();
        let error_message = self.error_message.clone();

        *is_loading.write().unwrap() = true;
        *error_message.write().unwrap() = None;

        spawn_local(async move {
            let ctx = context::context();

            match ctx.list_directory(&path).await {
                Ok(response) => {
                    // Update the tree node
                    let mut root_guard = root.write().unwrap();
                    if let Some(node) = root_guard.find_node_mut(&path) {
                        node.insert_entries(&response.entries);
                    } else {
                        // If this is the root path, update the root node directly
                        if path == root_guard.path {
                            root_guard.insert_entries(&response.entries);
                        }
                    }
                }
                Err(e) => {
                    *error_message.write().unwrap() = Some(format!("Failed to load directory: {}", e));
                }
            }

            *is_loading.write().unwrap() = false;
        });
    }

    /// Navigate to a directory (expand and load if needed)
    pub fn navigate_to(&self, path: &str) {
        // Find the node and expand it
        let mut root = self.root.write().unwrap();
        if let Some(node) = root.find_node_mut(path) {
            if node.is_directory && !node.loaded {
                drop(root); // Release the lock before async operation
                self.load_directory(path);
            } else if node.is_directory {
                node.expanded = !node.expanded;
            }
        }
    }

    /// Navigate to parent directory (for tree view, this collapses the current level)
    pub fn navigate_up(&self) {
        // In tree view, navigation up is handled by the tree structure itself
        // This method is kept for compatibility but doesn't need to do anything special
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
            // Header with navigation and controls
            ui.horizontal(|ui| {
                ui.label(RichText::new("📁 File Browser").color(MutantColors::TEXT_PRIMARY));

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

            // File tree
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    // Clone necessary data to avoid borrowing issues
                    let show_hidden = *self.show_hidden.read().unwrap();
                    let selected_file = self.selected_file.read().unwrap().clone();

                    let mut root = self.root.write().unwrap();
                    file_selected = Self::draw_tree_node_static(
                        ui,
                        &mut *root,
                        0,
                        show_hidden,
                        &selected_file,
                        &self.selected_file,
                        &self.is_loading,
                        &self.error_message
                    );

                    // Check for expanded but unloaded directories and load them
                    let paths_to_load = Self::collect_unloaded_expanded_paths(&*root);
                    drop(root); // Release the lock before async operations

                    for path in paths_to_load {
                        self.load_directory(&path);
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

    /// Draw a tree node and its children (static version to avoid borrowing issues)
    fn draw_tree_node_static(
        ui: &mut egui::Ui,
        node: &mut FilePickerNode,
        indent_level: usize,
        show_hidden: bool,
        selected_file: &Option<String>,
        selected_file_arc: &Arc<RwLock<Option<String>>>,
        is_loading: &Arc<RwLock<bool>>,
        error_message: &Arc<RwLock<Option<String>>>
    ) -> bool {
        let mut file_selected = false;

        // Skip hidden files if needed
        if !show_hidden && node.name.starts_with('.') {
            return false;
        }

        // Compact indentation for maximum space efficiency
        let indent_per_level = 12.0;
        let total_indent = indent_per_level * (indent_level as f32);

        ui.horizontal(|ui| {
            // Apply indentation
            ui.add_space(total_indent);

            if node.is_directory {
                // Directory node - use collapsing header
                let icon = if node.expanded { "📂" } else { "📁" };
                let text = RichText::new(format!("{} {}/", icon, node.name))
                    .size(12.0)
                    .color(MutantColors::ACCENT_ORANGE);

                let header = egui::CollapsingHeader::new(text)
                    .default_open(node.expanded)
                    .show(ui, |ui| {
                        // Draw children
                        let mut sorted_children: Vec<_> = node.children.iter_mut().collect();
                        sorted_children.sort_by(|(_, a), (_, b)| {
                            match (a.is_directory, b.is_directory) {
                                (true, false) => std::cmp::Ordering::Less,
                                (false, true) => std::cmp::Ordering::Greater,
                                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                            }
                        });

                        for (_, child) in sorted_children {
                            if Self::draw_tree_node_static(
                                ui,
                                child,
                                indent_level + 1,
                                show_hidden,
                                selected_file,
                                selected_file_arc,
                                is_loading,
                                error_message
                            ) {
                                file_selected = true;
                            }
                        }
                    });

                // Handle directory expansion
                if header.header_response.clicked() {
                    node.expanded = header.openness > 0.0;
                    // Note: Directory loading will be handled by the main FilePicker logic
                    // when it detects an expanded but unloaded directory
                }
            } else {
                // File node
                ui.add_space(18.0); // Align with directory names

                let is_selected = selected_file
                    .as_ref()
                    .map_or(false, |selected| selected == &node.path);

                // File icon and color
                let file_icon = "📄";
                let filename_color = if is_selected {
                    MutantColors::ACCENT_ORANGE
                } else {
                    MutantColors::TEXT_PRIMARY
                };

                // Create clickable area for the file
                let row_response = ui.allocate_response(
                    egui::Vec2::new(ui.available_width(), 20.0),
                    egui::Sense::click()
                );

                let row_rect = row_response.rect;

                // Selection background
                if is_selected {
                    ui.painter().rect_filled(
                        row_rect,
                        4.0,
                        MutantColors::ACCENT_ORANGE.gamma_multiply(0.2)
                    );
                }

                // Draw file icon and name
                let text_pos = row_rect.left_top() + egui::Vec2::new(4.0, (row_rect.height() - 12.0) / 2.0);
                let font_id = egui::FontId::new(12.0, egui::FontFamily::Proportional);

                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    format!("{} {}", file_icon, node.name),
                    font_id,
                    filename_color
                );

                // File stats on the right
                if let Some(size) = node.size {
                    let size_text = format_file_size(size);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(size_text)
                                .color(MutantColors::TEXT_MUTED)
                                .size(11.0)
                        );
                    });
                }

                // Handle file selection
                if row_response.clicked() {
                    *selected_file_arc.write().unwrap() = Some(node.path.clone());
                    file_selected = true;
                }

                if row_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
        });

        file_selected
    }

    /// Collect paths of directories that are expanded but not loaded
    fn collect_unloaded_expanded_paths(node: &FilePickerNode) -> Vec<String> {
        let mut paths = Vec::new();

        if node.is_directory && node.expanded && !node.loaded {
            paths.push(node.path.clone());
        }

        // Recursively check children
        for child in node.children.values() {
            paths.extend(Self::collect_unloaded_expanded_paths(child));
        }

        paths
    }
}


