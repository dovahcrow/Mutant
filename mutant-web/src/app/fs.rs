use std::sync::{Arc, RwLock};
use std::collections::BTreeMap;

use eframe::egui::{self, Color32, RichText};
use humansize::{format_size, BINARY};
use mutant_protocol::KeyDetails;
use serde::{Deserialize, Serialize};

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
    /// Returns true if a file in this node or its children was clicked
    fn ui(&mut self, ui: &mut egui::Ui, indent_level: usize) -> bool {
        // Use a very small fixed indentation to avoid exponential growth
        let indent_per_level = 2.0;
        let total_indent = indent_per_level * (indent_level as f32);

        let mut file_clicked = false;

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

                let mut file_clicked = false;

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
                        if child.ui(ui, indent_level + 1) {
                            file_clicked = true;
                        }
                    }
                }).header_response.clicked() || self.expanded;

                // Update file_clicked with any clicks from children
                // but don't return early
            } else {
                // File node - add extra space to align with folder names (accounting for arrow)
                ui.add_space(18.0); // Add space to compensate for the arrow in front of folders

                let icon = "📄";
                let details = self.key_details.as_ref().unwrap();

                let text = if details.is_public {
                    RichText::new(format!("{} {} (public)", icon, self.name))
                        .color(Color32::from_rgb(0, 128, 0))
                } else {
                    RichText::new(format!("{} {}", icon, self.name))
                };

                // Make the file node clickable
                if ui.selectable_label(false, text).clicked() {
                    // Set file_clicked to true
                    file_clicked = true;
                }
            }

        });

        // Return whether a file was clicked
        file_clicked
    }
}

/// The filesystem tree window
#[derive(Clone, Serialize, Deserialize)]
pub struct FsWindow {
    keys: Arc<RwLock<Vec<KeyDetails>>>,
    root: TreeNode,
    /// Currently selected file (if any)
    selected_file: Option<KeyDetails>,
    /// Content of the selected file (placeholder for now)
    file_content: String,
}

impl Default for FsWindow {
    fn default() -> Self {
        Self {
            keys: crate::app::context::context().get_key_cache(),
            root: TreeNode::default(),
            selected_file: None,
            file_content: String::new(),
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
    fn build_tree(&mut self) {
        // Only rebuild if needed
        if self.root.children.is_empty() {
            let keys = self.keys.read().unwrap();

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
        }
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

            // Track clicked nodes to handle after the loop
            let mut clicked_node: Option<KeyDetails> = None;

            // Draw the sorted children with indentation and check if any file was clicked
            for (_, child) in sorted_children {
                if child.ui(ui, 1) { // Start with indent level 1 since we're inside root
                    // A file was clicked, get its details
                    if let Some(details) = &child.key_details {
                        clicked_node = Some(details.clone());
                    }
                }
            }

            // Handle the clicked node outside the loop to avoid borrow issues
            if let Some(details) = clicked_node {
                self.selected_file = Some(details.clone());

                // For now, just set a placeholder content
                self.file_content = format!(
                    "Content of file: {}\n\nThis is a placeholder. Actual file content will be implemented in a future task.",
                    details.key
                );
            }
        });
    }

    /// Draw the file viewer/editor panel
    fn draw_file_viewer(&mut self, ui: &mut egui::Ui) {
        if let Some(file) = &self.selected_file {
            // Show file details at the top
            ui.heading(&file.key);

            ui.horizontal(|ui| {
                ui.label(format!("Size: {}", humanize_size(file.total_size)));
                ui.separator();
                ui.label(format!("Pads: {}/{}", file.confirmed_pads, file.pad_count));
                ui.separator();
                if file.is_public {
                    ui.label(RichText::new("Public").color(Color32::from_rgb(0, 128, 0)));
                    if let Some(addr) = &file.public_address {
                        ui.separator();
                        ui.label(format!("Address: {}", addr));
                    }
                } else {
                    ui.label("Private");
                }
            });

            ui.separator();

            // File content area
            egui::ScrollArea::vertical().show(ui, |ui| {
                let text_edit = egui::TextEdit::multiline(&mut self.file_content)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace);

                ui.add(text_edit);
            });
        } else {
            // No file selected
            ui.centered_and_justified(|ui| {
                ui.heading("Select a file to view its content");
            });
        }
    }
}
