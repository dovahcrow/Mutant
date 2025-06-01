use std::collections::BTreeMap;
use eframe::egui::{self, RichText};
use mutant_protocol::KeyDetails;
use serde::{Deserialize, Serialize};
use crate::app::theme;
use crate::app::components::multimedia; // For get_file_icon_and_color
use humansize::{format_size, BINARY}; // For humanize_size

/// Helper function to format file sizes in a human-readable way
fn humanize_size(size: usize) -> String {
    format_size(size, BINARY)
}

/// A node in the filesystem tree
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreeNode {
    /// Name of this node (file or directory name)
    pub name: String,

    /// If this is a file, contains the key details
    pub key_details: Option<KeyDetails>,

    /// If this is a directory, contains child nodes
    pub children: BTreeMap<String, TreeNode>,

    /// Whether this directory is expanded (only relevant for directories)
    pub expanded: bool,

    /// Full path to this node (for debugging and unique IDs)
    pub path: String,
}

impl TreeNode {
    /// Create a new directory node
    pub fn new_dir(name: &str) -> Self {
        Self {
            name: name.to_string(),
            key_details: None,
            children: BTreeMap::new(),
            expanded: false,
            path: name.to_string(),
        }
    }

    /// Create a new file node
    pub fn new_file(name: &str, key_details: KeyDetails) -> Self {
        Self {
            name: name.to_string(),
            key_details: Some(key_details),
            children: BTreeMap::new(),
            expanded: false,
            path: name.to_string(),
        }
    }

    /// Check if this node is a directory
    pub fn is_dir(&self) -> bool {
        self.key_details.is_none()
    }

    /// Insert a key into the tree at the appropriate location
    pub fn insert_key(&mut self, path_parts: &[&str], key_details: KeyDetails, parent_path: &str) {
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
    /// Returns (view_clicked_details, download_clicked_details)
    pub fn ui(&mut self, ui: &mut egui::Ui, indent_level: usize, selected_path: Option<&str>, window_id: &str) -> (Option<KeyDetails>, Option<KeyDetails>) {
        // Extremely compact indentation for maximum space efficiency
        let indent_per_level = 1.5;  // Reduced from 3.0 to 1.5
        let total_indent = indent_per_level * (indent_level as f32);

        let mut view_clicked_details = None;
        let mut download_clicked_details = None;

        // Add subtle vertical spacing between items
        if indent_level > 0 {
            ui.add_space(2.0);
        }

        ui.horizontal(|ui| {
            // Draw visual delimiter line for tree structure
            if indent_level > 0 {
                let line_color = theme::MutantColors::BORDER_LIGHT;
                let line_x = total_indent - 6.0; // Position line slightly left of content
                let line_start = ui.cursor().top();
                let line_end = line_start + 20.0; // Height of one row

                ui.painter().line_segment(
                    [egui::Pos2::new(line_x, line_start), egui::Pos2::new(line_x, line_end)],
                    egui::Stroke::new(1.0, line_color)
                );
            }

            // Apply the base indentation
            ui.add_space(total_indent);

            if self.is_dir() {
                // Directory node - use collapsing header for proper tree behavior
                let icon = if self.expanded { "📂" } else { "📁" };
                let text = RichText::new(format!("{} {}/", icon, self.name))
                    .size(12.0)  // Match file font size for consistency
                    .color(theme::MutantColors::ACCENT_ORANGE);  // Special distinguishing color for folders

                let header = egui::CollapsingHeader::new(text)
                    .id_salt(format!("mutant_fs_{}dir_{}", window_id, self.path))
                    .default_open(self.expanded);

                let mut child_view_details = None;
                let mut child_download_details = None;

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
                        let (view_details, down_details) = child.ui(ui, indent_level + 1, selected_path, window_id);
                        if view_details.is_some() {
                            child_view_details = view_details;
                        }
                        if down_details.is_some() {
                            child_download_details = down_details;
                        }
                    }
                }).header_response.clicked() || self.expanded;

                // Propagate click from children
                if child_view_details.is_some() {
                    view_clicked_details = child_view_details;
                }
                if child_download_details.is_some() {
                    download_clicked_details = child_download_details;
                }
            } else {
                // File node - add extra space to align with folder names (accounting for arrow)
                ui.add_space(18.0); // Add space to compensate for the arrow in front of folders

                let details = self.key_details.as_ref().unwrap();
                let is_selected = selected_path.map_or(false, |path| path == &details.key);

                // Determine file icon and color based on extension
                let (file_icon, icon_color) = self.get_file_icon_and_color();

                // Style the filename text - less bright color and smaller font
                let filename_color = if details.is_public {
                    theme::MutantColors::SUCCESS
                } else {
                    theme::MutantColors::TEXT_SECONDARY  // Less bright than TEXT_PRIMARY
                };

                // Create file display text with icon
                // We'll draw the text manually for better control over fade effects

                // Make the file node clickable for viewing with proper layout
                let row_response = ui.allocate_response(
                    egui::Vec2::new(ui.available_width(), 20.0),
                    egui::Sense::click()
                );

                let row_rect = row_response.rect;

                // Add selection background for the entire row
                if is_selected {
                    ui.painter().rect_filled(
                        row_rect,
                        4.0,
                        theme::MutantColors::SELECTION
                    );
                }

                // Calculate metadata width and positioning
                let metadata_width = 100.0; // Balanced width for metadata area

                // Calculate text clipping area to prevent text from bleeding into the metadata area
                let fade_width = 40.0; // Balanced fade width for smooth gradient
                let text_clip_width = row_rect.width() - metadata_width - fade_width;

                // Create a clipping rectangle for the text to prevent bleeding
                let text_clip_rect = egui::Rect::from_min_size(
                    row_rect.left_top(),
                    egui::Vec2::new(text_clip_width, row_rect.height())
                );

                // Draw the icon and filename with different colors and smaller font
                // Center the text vertically in the row
                let text_pos = row_rect.left_top() + egui::Vec2::new(4.0, (row_rect.height() - 12.0) / 2.0);
                let font_id = egui::FontId::new(12.0, egui::FontFamily::Proportional);  // Smaller font size

                // Draw icon with its specific color
                let icon_galley = ui.painter().layout_no_wrap(
                    file_icon.to_string(),
                    font_id.clone(),
                    icon_color
                );
                let icon_width = icon_galley.rect.width() + 4.0; // Get width before moving
                ui.painter().with_clip_rect(text_clip_rect).galley(text_pos, icon_galley, icon_color);

                // Draw filename with less bright color, positioned after the icon
                let filename_pos = text_pos + egui::Vec2::new(icon_width, 0.0);
                let filename_galley = ui.painter().layout_no_wrap(
                    format!(" {}", self.name),
                    font_id.clone(),
                    filename_color
                );
                ui.painter().with_clip_rect(text_clip_rect).galley(filename_pos, filename_galley, filename_color);

                // Handle click on the entire row
                if row_response.clicked() {
                    view_clicked_details = Some(details.clone());
                }

                if row_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                // Draw metadata manually at the exact same position as text
                let text_baseline_y = row_rect.top() + (row_rect.height() - 12.0) / 2.0;

                // Calculate positions for metadata elements from right to left
                let mut current_x = row_rect.right() - 4.0; // Start with right padding

                // Download button position
                let button_width = 20.0;
                let button_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(current_x - button_width, text_baseline_y),
                    egui::Vec2::new(button_width, 12.0)
                );

                // Draw download button manually
                let button_response = ui.allocate_rect(button_rect, egui::Sense::click());
                if button_response.clicked() {
                    download_clicked_details = Some(details.clone());
                }
                if button_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                // Draw the arrow icon at the exact text baseline
                let arrow_galley = ui.painter().layout_no_wrap(
                    "⬇".to_string(),
                    egui::FontId::new(12.0, egui::FontFamily::Proportional),
                    theme::MutantColors::ACCENT_GREEN
                );
                ui.painter().galley(
                    egui::Pos2::new(current_x - button_width / 2.0 - arrow_galley.rect.width() / 2.0, text_baseline_y),
                    arrow_galley,
                    theme::MutantColors::ACCENT_GREEN
                );

                current_x -= button_width + 8.0; // Move left for next element

                // File size text
                let size_text = humanize_size(details.total_size);
                let size_galley = ui.painter().layout_no_wrap(
                    size_text,
                    egui::FontId::new(11.0, egui::FontFamily::Proportional),
                    theme::MutantColors::TEXT_MUTED
                );
                current_x -= size_galley.rect.width();
                ui.painter().galley(
                    egui::Pos2::new(current_x, text_baseline_y),
                    size_galley,
                    theme::MutantColors::TEXT_MUTED
                );

                // Public indicator if needed
                if details.is_public {
                    current_x -= 4.0; // Small spacing
                    let pub_galley = ui.painter().layout_no_wrap(
                        "PUB".to_string(),
                        egui::FontId::new(10.0, egui::FontFamily::Proportional),
                        theme::MutantColors::SUCCESS
                    );
                    current_x -= pub_galley.rect.width();
                    ui.painter().galley(
                        egui::Pos2::new(current_x, text_baseline_y),
                        pub_galley,
                        theme::MutantColors::SUCCESS
                    );
                }
            }
        });

        (view_clicked_details, download_clicked_details)
    }

    /// Get appropriate icon and color for file based on extension
    pub fn get_file_icon_and_color(&self) -> (&'static str, egui::Color32) {
        if let Some(extension) = std::path::Path::new(&self.name).extension() {
            match extension.to_string_lossy().to_lowercase().as_str() {
                // Code files - bright colors for different languages
                "rs" | "rust" => ("🦀", theme::MutantColors::ACCENT_ORANGE),
                "js" | "ts" | "jsx" | "tsx" => ("📜", theme::MutantColors::WARNING),
                "py" | "python" => ("🐍", theme::MutantColors::ACCENT_GREEN),
                "java" | "class" => ("☕", theme::MutantColors::ACCENT_ORANGE),
                "cpp" | "c" | "cc" | "cxx" | "h" | "hpp" => ("⚙️", theme::MutantColors::ACCENT_BLUE),
                "go" => ("🐹", theme::MutantColors::ACCENT_CYAN),
                "php" => ("🐘", theme::MutantColors::ACCENT_PURPLE),
                "rb" | "ruby" => ("💎", theme::MutantColors::ERROR),
                "swift" => ("🦉", theme::MutantColors::ACCENT_ORANGE),
                "kt" | "kotlin" => ("🎯", theme::MutantColors::ACCENT_PURPLE),
                "cs" | "csharp" => ("🔷", theme::MutantColors::ACCENT_BLUE),
                "html" | "htm" => ("🌐", theme::MutantColors::ACCENT_ORANGE),
                "css" | "scss" | "sass" | "less" => ("🎨", theme::MutantColors::ACCENT_BLUE),
                "json" | "yaml" | "yml" | "toml" | "xml" => ("📋", theme::MutantColors::TEXT_MUTED),

                // Images - green tones
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" => ("📷", theme::MutantColors::ACCENT_GREEN),

                // Videos - purple/magenta tones
                "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => ("🎬", theme::MutantColors::ACCENT_PURPLE),

                // Audio - cyan tones
                "mp3" | "wav" | "flac" | "ogg" | "aac" | "m4a" => ("🎵", theme::MutantColors::ACCENT_CYAN),

                // Archives - orange tones
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => ("📦", theme::MutantColors::ACCENT_ORANGE),

                // Documents - different colors for different types
                "pdf" => ("📕", theme::MutantColors::ERROR),
                "doc" | "docx" => ("📘", theme::MutantColors::ACCENT_BLUE),
                "xls" | "xlsx" => ("📗", theme::MutantColors::ACCENT_GREEN),
                "ppt" | "pptx" => ("📙", theme::MutantColors::WARNING),
                "txt" | "md" | "readme" => ("📄", theme::MutantColors::TEXT_MUTED),

                // Executables - bright warning colors
                "exe" | "msi" | "deb" | "rpm" | "dmg" | "app" => ("⚡", theme::MutantColors::WARNING),

                _ => ("📄", theme::MutantColors::TEXT_MUTED)
            }
        } else {
            ("📄", theme::MutantColors::TEXT_MUTED)
        }
    }
}
