use std::sync::{Arc, RwLock};
use std::collections::BTreeMap;

use eframe::egui::{self, Color32, RichText};
use humansize::{format_size, BINARY};
use mutant_protocol::KeyDetails;
use serde::{Deserialize, Serialize};
use base64::Engine;

use super::components::multimedia;
use super::Window;

/// Helper function to format file sizes in a human-readable way
fn humanize_size(size: usize) -> String {
    format_size(size, BINARY)
}

/// A node in the filesystem tree
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TreeNode {
    /// Name of this node (file or directory name)
    name: String,

    /// If this is a file, contains the key details
    key_details: Option<KeyDetails>,

    /// If this is a directory, contains child nodes
    children: BTreeMap<String, TreeNode>,

    /// Whether this directory is expanded (only relevant for directories)
    expanded: bool,

    /// Full path to this node (for debugging and unique IDs)
    path: String,
}

impl TreeNode {
    /// Create a new directory node
    fn new_dir(name: &str) -> Self {
        Self {
            name: name.to_string(),
            key_details: None,
            children: BTreeMap::new(),
            expanded: false,
            path: name.to_string(),
        }
    }

    /// Create a new file node
    fn new_file(name: &str, key_details: KeyDetails) -> Self {
        Self {
            name: name.to_string(),
            key_details: Some(key_details),
            children: BTreeMap::new(),
            expanded: false,
            path: name.to_string(),
        }
    }

    /// Check if this node is a directory
    fn is_dir(&self) -> bool {
        self.key_details.is_none()
    }

    /// Insert a key into the tree at the appropriate location
    fn insert_key(&mut self, path_parts: &[&str], key_details: KeyDetails, parent_path: &str) {
        if path_parts.is_empty() {
            return;
        }

        let current = path_parts[0];
        let current_path = if parent_path.is_empty() {
            current.to_string()
        } else {
            format!("{}/{}", parent_path, current)
        };

        if path_parts.len() == 1 {
            // This is a file (leaf node)
            let mut file_node = TreeNode::new_file(current, key_details);
            file_node.path = current_path;
            self.children.insert(current.to_string(), file_node);
        } else {
            // This is a directory
            let dir_node = self.children
                .entry(current.to_string())
                .or_insert_with(|| {
                    let mut node = TreeNode::new_dir(current);
                    node.path = current_path.clone();
                    node
                });

            dir_node.insert_key(&path_parts[1..], key_details, &current_path);
        }
    }

    /// Draw this node and its children
    /// Returns (clicked, key_details) if a file in this node or its children was clicked
    fn ui(&mut self, ui: &mut egui::Ui, indent_level: usize, selected_path: Option<&str>) -> (bool, Option<KeyDetails>) {
        // Use a very small fixed indentation to avoid exponential growth
        let indent_per_level = 2.0;
        let total_indent = indent_per_level * (indent_level as f32);

        let mut file_clicked = false;
        let mut clicked_details = None;

        ui.horizontal(|ui| {
            // Apply the base indentation
            ui.add_space(total_indent);

            if self.is_dir() {
                // Directory node
                let icon = if self.expanded { "📂" } else { "📁" };
                let text = format!("{} {}/", icon, self.name); // Add '/' at the end of folder names

                let header = egui::CollapsingHeader::new(text)
                    .id_salt(format!("dir_{}", self.path)) // Use full path for unique ID
                    .default_open(self.expanded);

                let mut child_clicked = false;
                let mut child_details = None;

                self.expanded = header.show(ui, |ui| {
                    // Sort children: directories first, then files
                    let mut sorted_children: Vec<_> = self.children.iter_mut().collect();
                    sorted_children.sort_by(|(_, a), (_, b)| {
                        match (a.is_dir(), b.is_dir()) {
                            (true, false) => std::cmp::Ordering::Less,    // Directories come before files
                            (false, true) => std::cmp::Ordering::Greater, // Files come after directories
                            _ => a.name.cmp(&b.name),                     // Sort alphabetically within each group
                        }
                    });

                    // Draw the sorted children with exactly one more level of indentation
                    for (_, child) in sorted_children {
                        let (clicked, details) = child.ui(ui, indent_level + 1, selected_path);
                        if clicked {
                            child_clicked = true;
                            child_details = details;
                        }
                    }
                }).header_response.clicked() || self.expanded;

                // Propagate click from children
                if child_clicked {
                    file_clicked = true;
                    clicked_details = child_details;
                }
            } else {
                // File node - add extra space to align with folder names (accounting for arrow)
                ui.add_space(18.0); // Add space to compensate for the arrow in front of folders

                let icon = "📄";
                let details = self.key_details.as_ref().unwrap();

                // Check if this file is selected
                let is_selected = selected_path.map_or(false, |path| path == &details.key);

                // Style the text based on selection and public status
                let text = if details.is_public {
                    let text = RichText::new(format!("{} {} (public)", icon, self.name))
                        .color(Color32::from_rgb(0, 128, 0));

                    if is_selected {
                        // text = text.strong().background_color(Color32::from_rgb(30, 30, 30));
                    }

                    text
                } else {
                    let text = RichText::new(format!("{} {}", icon, self.name));

                    // Commented out selection styling
                    // let text = if is_selected {
                    //     text.strong().background_color(Color32::from_rgb(30, 30, 30))
                    // } else {
                    //     text
                    // };

                    text
                };

                // Make the file node clickable
                if ui.selectable_label(is_selected, text).clicked() {
                    // Set file_clicked to true and return the details
                    file_clicked = true;
                    clicked_details = Some(details.clone());
                }
            }
        });

        // Return whether a file was clicked and the details if available
        (file_clicked, clicked_details)
    }
}

/// The filesystem tree window
#[derive(Clone, Serialize, Deserialize)]
pub struct FsWindow {
    keys: Arc<RwLock<Vec<KeyDetails>>>,
    root: TreeNode,
    /// Currently selected file (if any)
    selected_file: Option<KeyDetails>,
    /// Path of the selected file (for highlighting in the tree)
    selected_path: Option<String>,
    /// Content of the selected file (placeholder for now)
    file_content: String,
    /// Whether we're currently loading file content
    is_loading: bool,
    /// File content as binary data for multimedia
    #[serde(skip)]
    file_binary: Option<Vec<u8>>,
    /// Current file type
    #[serde(skip)]
    file_type: Option<multimedia::FileType>,
    /// Image texture for image files
    #[serde(skip)]
    image_texture: Option<eframe::egui::TextureHandle>,
    /// Video URL for video files
    #[serde(skip)]
    video_url: Option<String>,
    /// Whether the file content has been modified
    #[serde(skip)]
    file_modified: bool,
}

impl Default for FsWindow {
    fn default() -> Self {
        Self {
            keys: crate::app::context::context().get_key_cache(),
            root: TreeNode::default(),
            selected_file: None,
            selected_path: None,
            file_content: String::new(),
            is_loading: false,
            file_binary: None,
            file_type: None,
            image_texture: None,
            video_url: None,
            file_modified: false,
        }
    }
}

impl Window for FsWindow {
    fn name(&self) -> String {
        "MutAnt Files".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.build_tree();

        // Use a horizontal layout with two panels
        egui::SidePanel::left("file_tree_panel")
            .resizable(true)
            .min_width(200.0)
            .default_width(300.0)
            .show_inside(ui, |ui| {
                self.draw_tree(ui);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.draw_file_viewer(ui);
        });
    }
}

impl FsWindow {
    pub fn new() -> Self {
        Default::default()
    }

    /// Build the tree from the current keys
    /// If force_rebuild is true, the tree will be rebuilt even if it's not empty
    fn build_tree(&mut self) {
        // Get the current key count to check if we need to rebuild
        let keys = self.keys.read().unwrap();
        let current_key_count = keys.len();

        // Check if we need to rebuild the tree
        // Rebuild if:
        // 1. The tree is empty (first time or reset)
        // 2. The number of keys has changed (new keys added or removed)
        let needs_rebuild = self.root.children.is_empty() ||
                           self.count_tree_files(&self.root) != current_key_count;

        if needs_rebuild {
            log::info!("Rebuilding file tree with {} keys", current_key_count);

            // Create a fresh root
            self.root = TreeNode::new_dir("root");

            // Add each key to the tree
            for key in keys.iter() {
                let path = &key.key;

                // Ensure the path starts with a slash
                let normalized_path = if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/{}", path)
                };

                // Split the path into parts, skipping empty parts (consecutive slashes)
                let parts: Vec<&str> = normalized_path.split('/')
                    .filter(|part| !part.is_empty())
                    .collect();

                // Insert into the tree
                self.root.insert_key(&parts, key.clone(), "");
            }

            log::info!("File tree rebuilt successfully");
        }
    }

    /// Count the number of file nodes in the tree (for comparison with key count)
    fn count_tree_files(&self, node: &TreeNode) -> usize {
        let mut count = 0;

        // If this is a file node, count it
        if !node.is_dir() {
            count += 1;
        }

        // Count files in all children
        for (_, child) in &node.children {
            count += self.count_tree_files(child);
        }

        count
    }

    /// Draw the tree UI
    fn draw_tree(&mut self, ui: &mut egui::Ui) {
        ui.heading("File Explorer");

        ui.separator();

        // Draw the tree
        egui::ScrollArea::vertical().show(ui, |ui| {
            // First, show the root folder (always expanded)
            ui.horizontal(|ui| {
                // Root folder icon with zero indentation
                let icon = "📂";
                ui.label(format!("{} /", icon));
            });

            // Sort children: directories first, then files
            let mut sorted_children: Vec<_> = self.root.children.iter_mut().collect();
            sorted_children.sort_by(|(_, a), (_, b)| {
                match (a.is_dir(), b.is_dir()) {
                    (true, false) => std::cmp::Ordering::Less,    // Directories come before files
                    (false, true) => std::cmp::Ordering::Greater, // Files come after directories
                    _ => a.name.cmp(&b.name),                     // Sort alphabetically within each group
                }
            });

            // Get the currently selected path for highlighting
            let selected_path_ref = self.selected_path.as_deref();

            // Track clicked nodes to handle after the loop
            let mut clicked_details: Option<KeyDetails> = None;

            // Draw the sorted children with indentation and check if any file was clicked
            for (_, child) in sorted_children {
                let (clicked, details) = child.ui(ui, 1, selected_path_ref); // Start with indent level 1 since we're inside root
                if clicked && details.is_some() {
                    clicked_details = details;
                }
            }

            // Handle the clicked node outside the loop to avoid borrow issues
            if let Some(details) = clicked_details {
                self.selected_file = Some(details.clone());
                self.selected_path = Some(details.key.clone());

                // Fetch the file content
                let key = details.key.clone();
                let is_public = details.is_public;

                // Start loading
                self.is_loading = true;
                self.file_content = "Loading file content...".to_string();

                // Reset multimedia content
                self.file_binary = None;
                self.file_type = None;
                self.image_texture = None;
                self.video_url = None;

                // Spawn a task to fetch the content
                let window_id = ui.id();
                let ctx_clone = ui.ctx().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let ctx = crate::app::context::context();

                    // First try to get binary data
                    match ctx.get_file_binary(&key, is_public).await {
                        Ok(binary_data) => {
                            // Get a mutable reference to the window system
                            let mut window_system = crate::app::window_system::window_system_mut();

                            // Find our window and update its content
                            if let Some(window) = window_system.get_window_mut::<FsWindow>(window_id) {
                                // Store binary data
                                window.file_binary = Some(binary_data.clone());

                                // Detect file type
                                let file_type = multimedia::detect_file_type(&binary_data, &key);
                                window.file_type = Some(file_type.clone());

                                // Process based on file type
                                match file_type {
                                    multimedia::FileType::Text => {
                                        // Try to convert to text
                                        if let Ok(text) = String::from_utf8(binary_data) {
                                            window.file_content = text;
                                        } else {
                                            window.file_content = "Error: File contains binary data that cannot be displayed as text".to_string();
                                        }
                                    },
                                    multimedia::FileType::Code(lang) => {
                                        // Try to convert to text for code display
                                        if let Ok(text) = String::from_utf8(binary_data) {
                                            window.file_content = text;
                                            // Store the language in the file type for syntax highlighting
                                            window.file_type = Some(multimedia::FileType::Code(lang));
                                        } else {
                                            window.file_content = "Error: File contains binary data that cannot be displayed as code".to_string();
                                        }
                                    },
                                    multimedia::FileType::Image => {
                                        // Load image
                                        if let Some(texture) = multimedia::load_image(&ctx_clone, &binary_data) {
                                            window.image_texture = Some(texture);
                                            window.file_content = "Image file loaded successfully".to_string();
                                        } else {
                                            window.file_content = "Error: Failed to load image data".to_string();
                                        }
                                    },
                                    multimedia::FileType::Video => {
                                        // Create a data URL for the video
                                        let mime_type = mime_guess::from_path(&key).first_or_octet_stream().essence_str().to_string();
                                        let data_url = base64::engine::general_purpose::STANDARD.encode(&binary_data);
                                        window.video_url = Some(format!("data:{};base64,{}", mime_type, data_url));
                                        window.file_content = "Video file loaded successfully".to_string();
                                    },
                                    multimedia::FileType::Other => {
                                        // Try to convert to text anyway for viewing
                                        if let Ok(text) = String::from_utf8(binary_data) {
                                            window.file_content = text;
                                            // Change to text type for better viewing
                                            window.file_type = Some(multimedia::FileType::Text);
                                        } else {
                                            window.file_content = "This file type is not supported for viewing".to_string();
                                        }
                                    }
                                }

                                window.is_loading = false;
                            }
                        },
                        Err(e) => {
                            // Handle error
                            let mut window_system = crate::app::window_system::window_system_mut();
                            if let Some(window) = window_system.get_window_mut::<FsWindow>(window_id) {
                                window.file_content = format!("Error loading file content: {}", e);
                                window.is_loading = false;
                            }
                        }
                    }
                });
            }
        });
    }

    /// Save the modified file content
    fn save_file(&mut self, ui: &mut egui::Ui) {
        if let Some(file) = &self.selected_file.clone() {
            let key = file.key.clone();
            let is_public = file.is_public;
            let content = self.file_content.clone();

            // Start loading
            self.is_loading = true;

            // Reset the modified flag
            self.file_modified = false;

            // Spawn a task to save the file
            let window_id = ui.id();
            wasm_bindgen_futures::spawn_local(async move {
                let ctx = crate::app::context::context();

                // Convert the content to bytes
                let data = content.into_bytes();

                // Get the filename from the key
                let filename = std::path::Path::new(&key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());

                // Save the file using the context's put method with all required parameters
                match ctx.put(
                    &key,
                    data,
                    &filename,
                    mutant_protocol::StorageMode::Medium,
                    is_public,
                    false, // no_verify
                    None,  // progress
                ).await {
                    Ok(_) => {
                        // Get a mutable reference to the window system
                        let mut window_system = crate::app::window_system::window_system_mut();

                        // Find our window and update it
                        if let Some(window) = window_system.get_window_mut::<FsWindow>(window_id) {
                            window.is_loading = false;

                            // Refresh the key list
                            let _ = ctx.list_keys().await;
                            window.build_tree();
                        }
                    },
                    Err(e) => {
                        // Get a mutable reference to the window system
                        let mut window_system = crate::app::window_system::window_system_mut();

                        // Find our window and update it
                        if let Some(window) = window_system.get_window_mut::<FsWindow>(window_id) {
                            window.is_loading = false;
                            window.file_modified = true; // Keep the modified flag since save failed

                            // Show error message
                            window.file_content = format!("Error saving file: {}", e);
                        }
                    }
                }
            });
        }
    }

    /// Draw the file viewer/editor panel
    fn draw_file_viewer(&mut self, ui: &mut egui::Ui) {
        if let Some(file) = &self.selected_file.clone() {
            // Show file details at the top
            ui.heading(&file.key);

            // Store file details we need for display
            let file_size = humanize_size(file.total_size);
            let pad_count = file.pad_count;
            let confirmed_pads = file.confirmed_pads;
            let is_public = file.is_public;
            let public_address = file.public_address.clone();

            // Store current state
            let file_type = self.file_type.clone();
            let file_modified = self.file_modified;

            ui.horizontal(|ui| {
                ui.label(format!("Size: {}", file_size));
                ui.separator();
                ui.label(format!("Pads: {}/{}", confirmed_pads, pad_count));
                ui.separator();
                if is_public {
                    ui.label(RichText::new("Public").color(Color32::from_rgb(0, 128, 0)));
                    if let Some(addr) = &public_address {
                        ui.separator();
                        ui.label(format!("Address: {}", addr));
                    }
                } else {
                    ui.label("Private");
                }

                // Show file type if available
                if let Some(file_type) = &file_type {
                    ui.separator();
                    match file_type {
                        multimedia::FileType::Text => {
                            ui.label("Type: Text");
                        },
                        multimedia::FileType::Code(lang) => {
                            ui.label(format!("Type: Code ({})", lang));
                        },
                        multimedia::FileType::Image => {
                            ui.label("Type: Image");
                        },
                        multimedia::FileType::Video => {
                            ui.label("Type: Video");
                        },
                        multimedia::FileType::Other => {
                            ui.label("Type: Unknown");
                        },
                    }
                }

                // Show modified indicator and save button if the file has been modified
                if file_modified {
                    ui.separator();
                    ui.label(RichText::new("Modified").color(Color32::YELLOW));

                    if ui.button("Save").clicked() {
                        // Save the file
                        self.save_file(ui);
                    }
                }
            });

            ui.separator();

            // Show loading indicator if we're loading content
            if self.is_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading file content...");
                });
                return;
            }

            // Display content based on file type
            if let Some(file_type) = &self.file_type {
                match file_type {
                    multimedia::FileType::Text => {
                        // Text viewer with syntax highlighting
                        if let Some(new_content) = multimedia::draw_text_viewer(ui, &self.file_content) {
                            // Content was edited
                            self.file_content = new_content;
                            self.file_modified = true;
                        }
                    },
                    multimedia::FileType::Code(lang) => {
                        // Code viewer with syntax highlighting
                        if let Some(new_content) = multimedia::draw_code_viewer(ui, &self.file_content, lang) {
                            // Content was edited
                            self.file_content = new_content;
                            self.file_modified = true;
                        }
                    },
                    multimedia::FileType::Image => {
                        // Image viewer
                        if let Some(texture) = &self.image_texture {
                            multimedia::draw_image_viewer(ui, texture);
                        } else {
                            ui.label("Error: Image texture not loaded");
                        }
                    },
                    multimedia::FileType::Video => {
                        // Video player
                        if let Some(video_url) = &self.video_url {
                            multimedia::draw_video_player(ui, video_url);
                        } else {
                            ui.label("Error: Video URL not available");
                        }
                    },
                    multimedia::FileType::Other => {
                        // Unsupported file type
                        multimedia::draw_unsupported_file(ui);
                    },
                }
            } else {
                // Fallback to text display if file type is not determined
                // Use our text viewer with syntax highlighting
                if let Some(new_content) = multimedia::draw_text_viewer(ui, &self.file_content) {
                    // Content was edited
                    self.file_content = new_content;
                    self.file_modified = true;
                }
            }
        } else {
            // No file selected
            ui.centered_and_justified(|ui| {
                ui.heading("Select a file to view its content");
            });
        }
    }
}
