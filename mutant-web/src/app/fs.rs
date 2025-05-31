use std::sync::{Arc, RwLock, Mutex}; // Added Mutex
use std::collections::BTreeMap;

use eframe::egui::{self, RichText};

use humansize::{format_size, BINARY};
use mutant_protocol::{KeyDetails, TaskProgress, GetEvent, TaskId}; // Added TaskProgress, GetEvent, TaskId
use serde::{Deserialize, Serialize};
use lazy_static::lazy_static;


use super::components::multimedia;
use super::Window;

use super::window_system::generate_unique_dock_area_id;
use super::{put::PutWindow, stats::StatsWindow};
use crate::utils::download_utils::{self, JsFileHandleResult, JsSimpleResult};
use js_sys::Uint8Array;
use wasm_bindgen_futures::spawn_local;
use log::{info, error}; // Ensure log levels are imported
use serde_wasm_bindgen;

// Global reference to the main FsWindow for async task access
lazy_static! {
    static ref MAIN_FS_WINDOW: Arc<RwLock<Option<Arc<RwLock<FsWindow>>>>> = Arc::new(RwLock::new(None));
}

/// Set the global reference to the main FsWindow
pub fn set_main_fs_window(fs_window: Arc<RwLock<FsWindow>>) {
    *MAIN_FS_WINDOW.write().unwrap() = Some(fs_window);
}

/// Get a reference to the main FsWindow for async tasks
pub fn get_main_fs_window() -> Option<Arc<RwLock<FsWindow>>> {
    MAIN_FS_WINDOW.read().unwrap().clone()
}

/// Helper function to format file sizes in a human-readable way
fn humanize_size(size: usize) -> String {
    format_size(size, BINARY)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] // Added Serialize/Deserialize for potential state saving
enum DownloadStatus {
    PendingFilePicker,
    Initializing, // Stream started, JS writer being set up
    Downloading,
    Completed,
    Failed,
    Cancelled, // Not yet implemented, but good to have
}

#[derive(Clone)] // Clone needed if passing to spawned tasks directly, but better to use Arc<Mutex<>>
struct ActiveDownload {
    task_id: TaskId,
    file_name: String,
    key_details: KeyDetails,
    
    // Receivers will be handled by the spawned task, not stored directly here after spawning.
    // Instead, store the writer_id.
    js_writer_id: Option<String>,

    status: DownloadStatus,
    progress_val: f32,
    downloaded_bytes: u64,
    // total_bytes is in key_details.total_size
    error_message: Option<String>,
    
    // Used to signal the data processing task to stop if picker fails or user cancels.
    // This is a simple way to communicate between the picker task and processing tasks.
    stop_signal_tx: Option<Arc<tokio::sync::watch::Sender<bool>>>,
    stop_signal_rx: Option<Arc<tokio::sync::watch::Receiver<bool>>>,
}

impl ActiveDownload {
    fn new(task_id: TaskId, key_details: KeyDetails) -> Self {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        Self {
            task_id,
            file_name: std::path::Path::new(&key_details.key)
                .file_name().map_or_else(|| key_details.key.clone(), |f| f.to_string_lossy().into_owned()),
            key_details,
            js_writer_id: None,
            status: DownloadStatus::PendingFilePicker,
            progress_val: 0.0,
            downloaded_bytes: 0,
            error_message: None,
            stop_signal_tx: Some(Arc::new(stop_tx)),
            stop_signal_rx: Some(Arc::new(stop_rx)),
        }
    }
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
    /// Returns (view_clicked_details, download_clicked_details)
    fn ui(&mut self, ui: &mut egui::Ui, indent_level: usize, selected_path: Option<&str>, window_id: &str) -> (Option<KeyDetails>, Option<KeyDetails>) {
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
                let line_color = super::theme::MutantColors::BORDER_LIGHT;
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
                    .color(super::theme::MutantColors::ACCENT_ORANGE);  // Special distinguishing color for folders

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
                    super::theme::MutantColors::SUCCESS
                } else {
                    super::theme::MutantColors::TEXT_SECONDARY  // Less bright than TEXT_PRIMARY
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
                        super::theme::MutantColors::SELECTION
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
                    super::theme::MutantColors::ACCENT_GREEN
                );
                ui.painter().galley(
                    egui::Pos2::new(current_x - button_width / 2.0 - arrow_galley.rect.width() / 2.0, text_baseline_y),
                    arrow_galley,
                    super::theme::MutantColors::ACCENT_GREEN
                );

                current_x -= button_width + 8.0; // Move left for next element

                // File size text
                let size_text = humanize_size(details.total_size);
                let size_galley = ui.painter().layout_no_wrap(
                    size_text,
                    egui::FontId::new(11.0, egui::FontFamily::Proportional),
                    super::theme::MutantColors::TEXT_MUTED
                );
                current_x -= size_galley.rect.width();
                ui.painter().galley(
                    egui::Pos2::new(current_x, text_baseline_y),
                    size_galley,
                    super::theme::MutantColors::TEXT_MUTED
                );

                // Public indicator if needed
                if details.is_public {
                    current_x -= 4.0; // Small spacing
                    let pub_galley = ui.painter().layout_no_wrap(
                        "PUB".to_string(),
                        egui::FontId::new(10.0, egui::FontFamily::Proportional),
                        super::theme::MutantColors::SUCCESS
                    );
                    current_x -= pub_galley.rect.width();
                    ui.painter().galley(
                        egui::Pos2::new(current_x, text_baseline_y),
                        pub_galley,
                        super::theme::MutantColors::SUCCESS
                    );
                }
            }
        });

        (view_clicked_details, download_clicked_details)
    }

    /// Get appropriate icon and color for file based on extension
    fn get_file_icon_and_color(&self) -> (&'static str, egui::Color32) {
        if let Some(extension) = std::path::Path::new(&self.name).extension() {
            match extension.to_string_lossy().to_lowercase().as_str() {
                // Code files - bright colors for different languages
                "rs" | "rust" => ("🦀", super::theme::MutantColors::ACCENT_ORANGE),
                "js" | "ts" | "jsx" | "tsx" => ("📜", super::theme::MutantColors::WARNING),
                "py" | "python" => ("🐍", super::theme::MutantColors::ACCENT_GREEN),
                "java" | "class" => ("☕", super::theme::MutantColors::ACCENT_ORANGE),
                "cpp" | "c" | "cc" | "cxx" | "h" | "hpp" => ("⚙️", super::theme::MutantColors::ACCENT_BLUE),
                "go" => ("🐹", super::theme::MutantColors::ACCENT_CYAN),
                "php" => ("🐘", super::theme::MutantColors::ACCENT_PURPLE),
                "rb" | "ruby" => ("💎", super::theme::MutantColors::ERROR),
                "swift" => ("🦉", super::theme::MutantColors::ACCENT_ORANGE),
                "kt" | "kotlin" => ("🎯", super::theme::MutantColors::ACCENT_PURPLE),
                "cs" | "csharp" => ("🔷", super::theme::MutantColors::ACCENT_BLUE),
                "html" | "htm" => ("🌐", super::theme::MutantColors::ACCENT_ORANGE),
                "css" | "scss" | "sass" | "less" => ("🎨", super::theme::MutantColors::ACCENT_BLUE),
                "json" | "yaml" | "yml" | "toml" | "xml" => ("📋", super::theme::MutantColors::TEXT_MUTED),

                // Images - green tones
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" => ("📷", super::theme::MutantColors::ACCENT_GREEN),

                // Videos - purple/magenta tones
                "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => ("🎬", super::theme::MutantColors::ACCENT_PURPLE),

                // Audio - cyan tones
                "mp3" | "wav" | "flac" | "ogg" | "aac" | "m4a" => ("🎵", super::theme::MutantColors::ACCENT_CYAN),

                // Archives - orange tones
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => ("📦", super::theme::MutantColors::ACCENT_ORANGE),

                // Documents - different colors for different types
                "pdf" => ("📕", super::theme::MutantColors::ERROR),
                "doc" | "docx" => ("📘", super::theme::MutantColors::ACCENT_BLUE),
                "xls" | "xlsx" => ("📗", super::theme::MutantColors::ACCENT_GREEN),
                "ppt" | "pptx" => ("📙", super::theme::MutantColors::WARNING),
                "txt" | "md" | "readme" => ("📄", super::theme::MutantColors::TEXT_MUTED),

                // Executables - bright warning colors
                "exe" | "msi" | "deb" | "rpm" | "dmg" | "app" => ("⚡", super::theme::MutantColors::WARNING),

                _ => ("📄", super::theme::MutantColors::TEXT_MUTED)
            }
        } else {
            ("📄", super::theme::MutantColors::TEXT_MUTED)
        }
    }
}

/// A tab in the file viewer area
#[derive(Clone, Serialize, Deserialize)]
pub struct FileViewerTab {
    /// File details
    pub file: KeyDetails,
    /// Content of the file
    pub content: String,
    /// Whether we're currently loading file content
    pub is_loading: bool,
    /// File content as binary data for multimedia
    #[serde(skip)]
    pub file_binary: Option<Vec<u8>>,
    /// Current file type
    #[serde(skip)]
    pub file_type: Option<multimedia::FileType>,
    /// Image texture for image files
    #[serde(skip)]
    pub image_texture: Option<eframe::egui::TextureHandle>,
    /// Video URL for video files
    #[serde(skip)]
    pub video_url: Option<String>,
    /// Whether the file content has been modified
    #[serde(skip)]
    pub file_modified: bool,
    /// Processed file content for multimedia components
    #[serde(skip)]
    pub file_content: Option<multimedia::FileContent>,
    /// Progress tracking for file loading
    #[serde(skip)]
    pub loading_progress: f32, // 0.0 to 1.0
    /// Total bytes expected for the file
    #[serde(skip)]
    pub total_bytes: Option<usize>,
    /// Bytes downloaded so far
    #[serde(skip)]
    pub downloaded_bytes: usize,
    /// Preferred dock area ID (for smart placement)
    #[serde(skip)]
    pub preferred_dock_area: Option<String>,
}

impl FileViewerTab {
    /// Create a new file viewer tab
    pub fn new(file: KeyDetails) -> Self {
        log::info!("Creating new FileViewerTab for: {}", file.key);

        // Initial content while loading
        let initial_content = "Loading file content...".to_string();

        // Create the FileContent immediately with default values
        // This will be updated when the actual content is loaded
        let file_content = multimedia::FileContent {
            file_type: multimedia::FileType::Text, // Default to text
            raw_data: initial_content.as_bytes().to_vec(),
            editable_content: Some(initial_content.clone()),
            content_modified: false,
            image_texture: None,
            video_url: None,
        };

        let tab = Self {
            file,
            content: initial_content,
            is_loading: true,
            file_binary: None,
            file_type: None,
            image_texture: None,
            video_url: None,
            file_modified: false,
            file_content: Some(file_content),
            loading_progress: 0.0,
            total_bytes: None,
            downloaded_bytes: 0,
            preferred_dock_area: None,
        };

        log::info!("Successfully created FileViewerTab");
        tab
    }

    /// Draw the file viewer tab
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // Don't override the style - let it inherit from the root theme

        // Create a professional header section with dark styling
        let header_frame = egui::Frame::new()
            .fill(super::theme::MutantColors::BACKGROUND_MEDIUM)
            .stroke(egui::Stroke::new(1.0, super::theme::MutantColors::BORDER_DARK))
            .inner_margin(egui::Margin::same(8));

        header_frame.show(ui, |ui| {
            // Top row: File icon, name, and status indicators
            ui.horizontal(|ui| {
                // Get file icon and color using the same logic as filesystem tree
                let file_name = std::path::Path::new(&self.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.file.key.clone());

                let (file_icon, icon_color) = {
                    // Create a temporary TreeNode to reuse the icon logic
                    let temp_node = TreeNode::new_file(&file_name, self.file.clone());
                    temp_node.get_file_icon_and_color()
                };

                // File icon with matching color from filesystem tree
                ui.label(
                    egui::RichText::new(file_icon)
                        .size(16.0)
                        .color(icon_color)
                );

                ui.add_space(8.0);

                // File name as title with professional styling
                ui.label(
                    egui::RichText::new(&self.file.key)
                        .size(16.0)
                        .strong()
                        .color(super::theme::MutantColors::TEXT_PRIMARY)
                );

                // Add some space before status indicators
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Show modified indicator if the file has been modified
                    if self.file_modified {
                        ui.label(
                            egui::RichText::new("● Modified")
                                .size(11.0)
                                .color(super::theme::MutantColors::WARNING)
                        );

                        if ui.add(crate::app::theme::primary_button("Save")).clicked() {
                            // Save the file
                            self.save_file(ui);
                        }
                    }

                    // Public/Private status with colored indicator
                    if self.file.is_public {
                        ui.label(
                            egui::RichText::new("🌐 Public")
                                .size(11.0)
                                .color(super::theme::MutantColors::SUCCESS)
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("🔒 Private")
                                .size(11.0)
                                .color(super::theme::MutantColors::ACCENT_BLUE)
                        );
                    }
                });
            });

            ui.add_space(6.0);

            // Bottom row: Detailed metadata with colors
            ui.horizontal(|ui| {
                // Store file details we need for display
                let file_size = humanize_size(self.file.total_size);
                let pad_count = self.file.pad_count;
                let confirmed_pads = self.file.confirmed_pads;

                // Size info with icon
                ui.label(
                    egui::RichText::new("📏")
                        .size(10.0)
                        .color(super::theme::MutantColors::ACCENT_CYAN)
                );
                ui.label(
                    egui::RichText::new(format!("{}", file_size))
                        .size(11.0)
                        .color(super::theme::MutantColors::TEXT_SECONDARY)
                );

                ui.separator();

                // Pad info with icon and progress color
                ui.label(
                    egui::RichText::new("🧩")
                        .size(10.0)
                        .color(super::theme::MutantColors::ACCENT_PURPLE)
                );
                let pad_color = if confirmed_pads == pad_count {
                    super::theme::MutantColors::SUCCESS
                } else {
                    super::theme::MutantColors::WARNING
                };
                ui.label(
                    egui::RichText::new(format!("{}/{} pads", confirmed_pads, pad_count))
                        .size(11.0)
                        .color(pad_color)
                );

                ui.separator();

                // Show file type with colored icon
                if let Some(file_type) = &self.file_type {
                    let (type_icon, type_color) = match file_type {
                        multimedia::FileType::Text => ("📄", super::theme::MutantColors::TEXT_SECONDARY),
                        multimedia::FileType::Code(_) => ("📝", super::theme::MutantColors::ACCENT_GREEN),
                        multimedia::FileType::Image => ("🖼️", super::theme::MutantColors::ACCENT_CYAN),
                        multimedia::FileType::Video => ("🎬", super::theme::MutantColors::ACCENT_PURPLE),
                        multimedia::FileType::Other => ("❓", super::theme::MutantColors::TEXT_MUTED),
                    };

                    let type_text = match file_type {
                        multimedia::FileType::Text => "Text".to_string(),
                        multimedia::FileType::Code(lang) => format!("Code ({})", lang),
                        multimedia::FileType::Image => "Image".to_string(),
                        multimedia::FileType::Video => "Video".to_string(),
                        multimedia::FileType::Other => "Unknown".to_string(),
                    };

                    ui.label(
                        egui::RichText::new(type_icon)
                            .size(10.0)
                            .color(type_color)
                    );
                    ui.label(
                        egui::RichText::new(type_text)
                            .size(11.0)
                            .color(type_color)
                    );
                }

                // Show public address if available
                if let Some(addr) = &self.file.public_address {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("🔗")
                            .size(10.0)
                            .color(super::theme::MutantColors::ACCENT_BLUE)
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", &addr[..8]))
                            .size(10.0)
                            .color(super::theme::MutantColors::TEXT_MUTED)
                    );
                }
            });
        });

        ui.add_space(4.0);

        // Show loading indicator if we're loading content
        if self.is_loading {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading file content...");
                });

                // Show progress bar if we have progress information
                if self.loading_progress > 0.0 || self.total_bytes.is_some() {
                    ui.add_space(4.0);

                    // Show bytes downloaded if we have that info
                    if let Some(total) = self.total_bytes {
                        ui.label(format!(
                            "Downloaded {} of {} ({}%)",
                            humanize_size(self.downloaded_bytes),
                            humanize_size(total),
                            (self.loading_progress * 100.0) as u32
                        ));
                    } else if self.downloaded_bytes > 0 {
                        ui.label(format!("Downloaded {}", humanize_size(self.downloaded_bytes)));
                    }

                    ui.add_space(2.0);
                    let progress_bar = egui::ProgressBar::new(self.loading_progress)
                        .show_percentage()
                        .animate(true);
                    ui.add(progress_bar);
                }

                // Fallback: Check if we have a progress object for this file (legacy support)
                if self.loading_progress == 0.0 && self.total_bytes.is_none() {
                    let ctx = crate::app::context::context();
                    let get_id = format!("get_{}", self.file.key);

                    if let Some(progress) = ctx.get_get_progress(&get_id) {
                        let progress_guard = progress.read().unwrap();

                        // Check if we have a get operation in progress
                        if let Some(op) = progress_guard.operation.get("get") {
                            if op.total_pads > 0 {
                                // Calculate progress percentage
                                let progress_value = op.nb_reserved as f32 / op.total_pads as f32;

                                // Show progress bar
                                ui.add_space(4.0);
                                ui.label(format!("Downloaded {} of {} pads", op.nb_reserved, op.total_pads));
                                ui.add_space(4.0);
                                let progress_bar = egui::ProgressBar::new(progress_value)
                                    .show_percentage()
                                    .animate(true);
                                ui.add(progress_bar);
                            }
                        }
                    }
                }
            });

            return;
        }

        // We should never need to initialize file_content here
        // This is a fallback for backward compatibility only
        if self.file_content.is_none() {
            log::warn!("FileContent not initialized for tab: {}", self.file.key);
        }

        // Display content based on file type
        if let Some(file_type) = &self.file_type {
            match file_type {
                multimedia::FileType::Text => {
                    // Use the stored FileContent
                    if let Some(file_content) = &mut self.file_content {
                        // Text viewer with syntax highlighting
                        multimedia::draw_text_viewer(ui, file_content);

                        // Check if content was modified
                        if file_content.content_modified {
                            // Update our content directly from the editable_content
                            if let Some(text) = &file_content.editable_content {
                                self.content = text.clone();
                                self.file_modified = true;
                                file_content.content_modified = false;

                                // Update the raw_data to match the new content
                                // file_content.raw_data = self.content.as_bytes().to_vec();
                            }
                        }
                    } else {
                        ui.label("Error: File content not available");
                    }
                },
                multimedia::FileType::Code(lang) => {
                    // Use the stored FileContent
                    if let Some(file_content) = &mut self.file_content {
                        // Code viewer with syntax highlighting
                        multimedia::draw_code_viewer(ui, file_content, lang);

                        // Check if content was modified
                        if file_content.content_modified {
                            // Update our content directly from the editable_content
                            if let Some(text) = &file_content.editable_content {
                                self.content = text.clone();
                                self.file_modified = true;
                                file_content.content_modified = false;

                                // Update the raw_data to match the new content
                                // file_content.raw_data = self.content.as_bytes().to_vec();
                            }
                        }
                    } else {
                        ui.label("Error: File content not available");
                    }
                },
                multimedia::FileType::Image => {
                    // Image viewer - create texture if not already created
                    if self.image_texture.is_none() && self.file_binary.is_some() {
                        // Create the texture from binary data
                        if let Some(binary_data) = &self.file_binary {
                            self.image_texture = multimedia::load_image(ui.ctx(), binary_data);
                        }
                    }

                    if let Some(texture) = &self.image_texture {
                        multimedia::draw_image_viewer(ui, texture);
                    } else if self.file_binary.is_some() {
                        ui.label("Error: Failed to create image texture");
                    } else {
                        ui.label("Loading image...");
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
            if let Some(file_content) = &mut self.file_content {
                multimedia::draw_text_viewer(ui, file_content);

                // Check if content was modified
                if file_content.content_modified {
                    // Update our content directly from the editable_content
                    if let Some(text) = &file_content.editable_content {
                        self.content = text.clone();
                        self.file_modified = true;
                        file_content.content_modified = false;

                        // Update the raw_data to match the new content
                        // file_content.raw_data = self.content.as_bytes().to_vec();
                    }
                }
            } else {
                ui.label("No file content available");
            }
        }
    }

    /// Save the file content
    fn save_file(&mut self, ui: &mut egui::Ui) {
        let key = self.file.key.clone();
        let is_public = self.file.is_public;
        let content = self.content.clone();

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
                mutant_protocol::StorageMode::Heaviest,
                is_public,
                false, // no_verify
                None,  // progress
            ).await {
                Ok(_) => {
                    // Get a mutable reference to the window system
                    let mut window_system = crate::app::window_system::window_system_mut();

                    // Find the tab with this file and update it
                    if let Some(tab) = window_system.find_file_viewer_tab_mut(&key) {
                        tab.is_loading = false;
                    }

                    // Refresh the key list
                    let _ = ctx.list_keys().await;

                    // Find our window and rebuild tree
                    if let Some(window) = window_system.get_window_mut::<FsWindow>(window_id) {
                        window.build_tree();
                    }
                },
                Err(e) => {
                    // Get a mutable reference to the window system
                    let mut window_system = crate::app::window_system::window_system_mut();

                    // Find the tab with this file and update it
                    if let Some(tab) = window_system.find_file_viewer_tab_mut(&key) {
                        tab.is_loading = false;
                        tab.file_modified = true; // Keep the modified flag since save failed
                        tab.content = format!("Error saving file: {}", e);
                    }
                }
            }
        });
    }
}

/// Unified tab type for the FsWindow's internal dock system
#[derive(Clone, Serialize, Deserialize)]
pub enum FsInternalTab {
    FileViewer(FileViewerTab),
    Put(PutWindow),
    Stats(StatsWindow),
}

impl FsInternalTab {
    pub fn name(&self) -> String {
        match self {
            Self::FileViewer(tab) => {
                let file_name = std::path::Path::new(&tab.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| tab.file.key.clone());
                file_name
            }
            Self::Put(window) => window.name(),
            Self::Stats(window) => window.name(),
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        match self {
            Self::FileViewer(tab) => tab.draw(ui),
            Self::Put(window) => window.draw(ui),
            Self::Stats(window) => window.draw(ui),
        }
    }
}

/// TabViewer for FsInternalTab in the FsWindow's internal dock
struct FsInternalTabViewer {}

impl egui_dock::TabViewer for FsInternalTabViewer {
    type Tab = FsInternalTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            FsInternalTab::FileViewer(file_tab) => {
                // Get the file name from the path
                let file_name = std::path::Path::new(&file_tab.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_tab.file.key.clone());

                // Get file icon and color using the same logic as filesystem tree
                let (file_icon, _icon_color) = {
                    // Create a temporary TreeNode to reuse the icon logic
                    let temp_node = TreeNode::new_file(&file_name, file_tab.file.clone());
                    temp_node.get_file_icon_and_color()
                };

                // Add a modified indicator if the file has been modified
                let modified_indicator = if file_tab.file_modified { "* " } else { "" };

                let title = format!("{}{}{}", file_icon, modified_indicator, file_name);

                // Use default color - let the dock style handle it
                egui::RichText::new(title)
                    .size(12.0)
                    .into()
            }
            FsInternalTab::Put(_) => {
                egui::RichText::new("📤 Upload")
                    .size(12.0)
                    .into()
            }
            FsInternalTab::Stats(_) => {
                egui::RichText::new("📊 Stats")
                    .size(12.0)
                    .into()
            }
        }
    }





    fn on_close(&mut self, tab: &mut Self::Tab) -> bool {
        // Custom close button styling will be handled by egui_dock
        // Return true to allow closing
        match tab {
            FsInternalTab::FileViewer(file_tab) => {
                // You could add save confirmation here if the file is modified
                if file_tab.file_modified {
                    // For now, just allow closing - could add confirmation dialog
                    log::info!("Closing modified file: {}", file_tab.file.key);
                }
                true
            }
            _ => true,
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        // Set proper background for the content area
        let frame = egui::Frame::new()
            .fill(super::theme::MutantColors::BACKGROUND_DARK)
            .inner_margin(egui::Margin::same(8));

        frame.show(ui, |ui| {
            tab.draw(ui);
        });
    }

    // Allow all tabs to be closable
    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }
}

/// The filesystem tree window
#[derive(Clone, Serialize, Deserialize)]
pub struct FsWindow {
    keys: Arc<RwLock<Vec<KeyDetails>>>,
    root: TreeNode,
    /// Path of the selected file (for highlighting in the tree)
    selected_path: Option<String>,
    /// Internal dock system for all tabs within this window
    internal_dock: egui_dock::DockState<FsInternalTab>,
    #[serde(skip)] // active_downloads should not be serialized
    active_downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    /// Unique identifier for this window instance to avoid widget ID conflicts
    #[serde(skip)]
    window_id: String,
    /// Unique dock area ID for the internal dock area within this FsWindow
    #[serde(skip)]
    dock_area_id: String,
}

impl Default for FsWindow {
    fn default() -> Self {
        Self {
            keys: crate::app::context::context().get_key_cache(),
            root: TreeNode::default(),
            selected_path: None,
            internal_dock: egui_dock::DockState::new(vec![]),
            active_downloads: Arc::new(Mutex::new(Vec::new())),
            window_id: uuid::Uuid::new_v4().to_string(),
            dock_area_id: generate_unique_dock_area_id(),
        }
    }
}

impl Window for FsWindow {
    fn name(&self) -> String {
        "MutAnt Files".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.build_tree();

        // Use a horizontal layout with two panels - improved styling
        egui::SidePanel::left(format!("mutant_fs_tree_panel_{}", self.window_id))
            .resizable(true)
            .min_width(220.0)
            .default_width(320.0)
            .max_width(500.0)
            .show_separator_line(true)
            .show_inside(ui, |ui| {
                // Apply consistent background styling
                ui.style_mut().visuals.panel_fill = super::theme::MutantColors::BACKGROUND_MEDIUM;
                self.draw_tree(ui);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Show the internal dock system for all tabs
            if self.internal_dock.iter_all_tabs().next().is_none() {
                // Show instructions when no tabs are open - properly centered both horizontally and vertically
                let available_rect = ui.available_rect_before_wrap();

                // Use vertical centering with flexible spacing
                ui.vertical_centered(|ui| {
                    // Add flexible space to push content to vertical center
                    let content_height = 120.0; // Approximate height of our content
                    let top_space = (available_rect.height() - content_height) / 2.0;
                    if top_space > 0.0 {
                        ui.add_space(top_space);
                    }

                    // Main heading with dimmed color
                    ui.label(
                        egui::RichText::new("MutAnt Workspace")
                            .size(24.0)
                            .color(super::theme::MutantColors::TEXT_MUTED)
                    );

                    ui.add_space(15.0);

                    // Instructions with even more dimmed color
                    ui.label(
                        egui::RichText::new("Click on a file in the tree to open it here.")
                            .size(14.0)
                            .color(super::theme::MutantColors::TEXT_DISABLED)
                    );

                    ui.add_space(8.0);

                    ui.label(
                        egui::RichText::new("Use the left menu to open Upload or Stats windows.")
                            .size(14.0)
                            .color(super::theme::MutantColors::TEXT_DISABLED)
                    );

                    ui.add_space(8.0);

                    ui.label(
                        egui::RichText::new("All windows will dock in this area.")
                            .size(14.0)
                            .color(super::theme::MutantColors::TEXT_DISABLED)
                    );
                });
            } else {
                // FORCE egui style override directly in the UI context
                ui.style_mut().visuals.widgets.active.weak_bg_fill = super::theme::MutantColors::BACKGROUND_LIGHT;
                ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, super::theme::MutantColors::ACCENT_ORANGE);
                ui.style_mut().visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, super::theme::MutantColors::ACCENT_ORANGE);

                ui.style_mut().visuals.widgets.inactive.weak_bg_fill = super::theme::MutantColors::BACKGROUND_MEDIUM;
                ui.style_mut().visuals.widgets.inactive.bg_stroke = egui::Stroke::new(0.0, egui::Color32::TRANSPARENT);
                ui.style_mut().visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, super::theme::MutantColors::TEXT_MUTED);

                ui.style_mut().visuals.widgets.hovered.weak_bg_fill = super::theme::MutantColors::SURFACE_HOVER;
                ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, super::theme::MutantColors::ACCENT_ORANGE);
                ui.style_mut().visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, super::theme::MutantColors::TEXT_PRIMARY);

                // Create a custom tab viewer that handles colors
                let mut tab_viewer = FsInternalTabViewer {};

                // Show the internal dock area with a unique ID
                egui_dock::DockArea::new(&mut self.internal_dock)
                    .id(egui::Id::new(format!("fs_internal_dock_{}", self.dock_area_id)))
                    .show_inside(ui, &mut tab_viewer);
            }
        });

        // // Draw Active Downloads UI
        // ui.separator(); // Visually separate from file viewer
        // ui.heading("Active Downloads");
        // egui::ScrollArea::vertical().show(ui, |ui| {
        //     let mut downloads_guard = self.active_downloads.lock().unwrap();
        //     // Remove completed/failed downloads after a delay or via a clear button (not implemented here)
        //     // For now, just display them.
        //     for download in downloads_guard.iter() {
        //         ui.label(format!("File: {}", download.file_name));
        //         ui.label(format!("Status: {:?}", download.status));
        //         if download.key_details.total_size > 0 {
        //             ui.add(egui::ProgressBar::new(download.progress_val).show_percentage());
        //             ui.label(format!("{} / {} bytes", 
        //                 humansize::format_size(download.downloaded_bytes, humansize::BINARY), 
        //                 humansize::format_size(download.key_details.total_size, humansize::BINARY)
        //             ));
        //         } else {
        //              ui.label(format!("{} downloaded", humansize::format_size(download.downloaded_bytes, humansize::BINARY)));
        //         }
        //         if let Some(err) = &download.error_message {
        //             ui.colored_label(crate::app::theme::MutantColors::ERROR, format!("Error: {}", err));
        //         }
        //         ui.separator();
        //     }
        //     // Keep only non-completed/failed for now, or implement manual clearing
        //     downloads_guard.retain(|d| match d.status {
        //         DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled => false,
        //         _ => true,
        //     });
        // });
    }
}

impl FsWindow {
    pub fn new() -> Self {
        // Use default implementation which already includes unique window_id
        Self::default()
    }
    
    fn update_download_state_js_writer(
        downloads: Arc<Mutex<Vec<ActiveDownload>>>,
        task_id: TaskId,
        writer_id: String,
        new_status: DownloadStatus,
    ) {
        let mut downloads_guard = downloads.lock().unwrap();
        if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
            info!("Updating download {} js_writer_id to {}, status to {:?}", dl.file_name, writer_id, new_status);
            dl.js_writer_id = Some(writer_id);
            dl.status = new_status;
        }
    }

    fn update_download_progress_bytes(
        downloads: Arc<Mutex<Vec<ActiveDownload>>>,
        task_id: TaskId,
        downloaded: u64,
    ) {
        let mut downloads_guard = downloads.lock().unwrap();
        if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
            dl.downloaded_bytes = downloaded;
            if dl.key_details.total_size > 0 {
                dl.progress_val = downloaded as f32 / dl.key_details.total_size as f32;
            }
        }
    }
    
    fn update_download_status(
        downloads: Arc<Mutex<Vec<ActiveDownload>>>,
        task_id: TaskId,
        new_status: DownloadStatus,
        error_msg: Option<String>,
    ) {
        let mut downloads_guard = downloads.lock().unwrap();
        if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
            info!("Updating download {} status to {:?}, error: {:?}", dl.file_name, new_status, error_msg);
            dl.status = new_status;
            if let Some(msg) = error_msg {
                dl.error_message = Some(msg);
            }
        }
    }

    fn update_download_state_error(
        downloads: Arc<Mutex<Vec<ActiveDownload>>>,
        task_id: TaskId,
        error_message: String,
    ) {
        FsWindow::update_download_status(downloads, task_id, DownloadStatus::Failed, Some(error_message));
    }

    fn initiate_download(&mut self, key_details: KeyDetails, egui_ctx: egui::Context) {
        info!("Initiating download for key: {}", key_details.key);
        let context = crate::app::context::context(); // Get app context
        let key_clone = key_details.clone();
        let active_downloads_clone = Arc::clone(&self.active_downloads);

        spawn_local(async move {
            match context.start_streamed_get(&key_clone.key, key_clone.is_public).await {
                Ok((task_id, mut progress_receiver, mut data_receiver)) => {
                    info!("start_streamed_get successful for {}. Task ID: {}", key_clone.key, task_id);
                    let download_entry = ActiveDownload::new(task_id, key_clone.clone());
                    
                    // Store stop signal receiver for later use if picker fails
                    let stop_rx_for_picker_failure = download_entry.stop_signal_rx.as_ref().unwrap().clone();
                    let stop_tx_for_picker_failure = download_entry.stop_signal_tx.as_ref().unwrap().clone();

                    active_downloads_clone.lock().unwrap().push(download_entry.clone()); // Add to list early
                    egui_ctx.request_repaint();

                    // 1. Get FileSystemFileHandle and WritableStream from JS
                    let file_name_for_picker = download_entry.file_name.clone();
                    match download_utils::js_init_save_file(&file_name_for_picker).await {
                        Ok(js_val) => {
                            let result: JsFileHandleResult = serde_wasm_bindgen::from_value(js_val).unwrap_or_else(|e| {
                                error!("Failed to deserialize JsFileHandleResult: {:?}", e);
                                JsFileHandleResult { writer_id: None, error: Some(format!("Deserialization error: {:?}",e)) }
                            });
                            if let Some(err_msg) = result.error {
                                error!("js_init_save_file failed for {}: {}", file_name_for_picker, err_msg);
                                // Update status to failed, signal tasks to stop
                                let _ = stop_tx_for_picker_failure.send(true); 
                                FsWindow::update_download_state_error(active_downloads_clone, task_id, err_msg);
                                return;
                            }
                            if let Some(writer_id) = result.writer_id {
                                info!("Obtained JS writer_id: {} for {}", writer_id, file_name_for_picker);
                                // Update download entry with writer_id and status
                                FsWindow::update_download_state_js_writer(active_downloads_clone.clone(), task_id, writer_id.clone(), DownloadStatus::Downloading);

                                // --- Spawn data processing task ---
                                let data_active_downloads = Arc::clone(&active_downloads_clone);
                                let data_writer_id = writer_id.clone();
                                let data_task_id = task_id;
                                let data_egui_ctx = egui_ctx.clone();
                                let data_stop_rx = stop_rx_for_picker_failure.clone(); // Use the same stop signal
                                spawn_local(async move {
                                    let mut current_downloaded_bytes = 0;
                                    let mut data_stop_rx_clone = (*data_stop_rx).clone();
                                    loop {
                                        tokio::select! {
                                            changed_res = data_stop_rx_clone.changed() => {
                                                if changed_res.is_err() || *data_stop_rx.borrow() { // if channel closed or true received
                                                    info!("Data task for {} stopping due to signal.", data_task_id);
                                                    // If picker failed, file wasn't created, so abortFile might not be needed or could error.
                                                    // Only call abort if we know a file handle was successfully created and then cancelled.
                                                    // For picker failure, the status is already set to Failed.
                                                    // If this is a user cancellation *after* picker, then abortFile is appropriate.
                                                    // FsWindow::update_download_status(data_active_downloads, data_task_id, DownloadStatus::Failed, Some("Cancelled".to_string()));
                                                    break;
                                                }
                                            }
                                            chunk_result = data_receiver.recv() => {
                                                match chunk_result {
                                                    Some(Ok(data_chunk)) => {
                                                        if data_chunk.is_empty() { continue; } 
                                                        current_downloaded_bytes += data_chunk.len() as u64;
                                                        FsWindow::update_download_progress_bytes(data_active_downloads.clone(), data_task_id, current_downloaded_bytes);
                                                        
                                                        let js_array = Uint8Array::from(&data_chunk[..]);
                                                        match download_utils::js_write_chunk(&data_writer_id, js_array).await {
                                                            Ok(val) => {
                                                                let write_result: JsSimpleResult = serde_wasm_bindgen::from_value(val).unwrap_or_else(|e| JsSimpleResult{ error: Some(format!("Deserialize error: {:?}", e))});
                                                                if let Some(err_msg) = write_result.error {
                                                                    error!("js_write_chunk failed for {}: {}", data_writer_id, err_msg);
                                                                    let _ = download_utils::js_abort_file(&data_writer_id, "write error").await;
                                                                    FsWindow::update_download_state_error(data_active_downloads, data_task_id, err_msg);
                                                                    break;
                                                                }
                                                            }
                                                            Err(e_write) => {
                                                                let err_msg = format!("js_write_chunk call failed: {:?}", e_write);
                                                                error!("{}", err_msg);
                                                                let _ = download_utils::js_abort_file(&data_writer_id, "write error").await;
                                                                FsWindow::update_download_state_error(data_active_downloads, data_task_id, err_msg);
                                                                break;
                                                            }
                                                        }
                                                        data_egui_ctx.request_repaint();
                                                    }
                                                    Some(Err(e)) => {
                                                        let err_msg = format!("Data stream error: {}", e);
                                                        error!("{}", err_msg);
                                                        let _ = download_utils::js_abort_file(&data_writer_id, "stream error").await;
                                                        FsWindow::update_download_state_error(data_active_downloads, data_task_id, err_msg);
                                                        break;
                                                    }
                                                    None => { // Stream ended
                                                        info!("Data stream ended for {}. Closing file.", data_task_id);
                                                        match download_utils::js_close_file(&data_writer_id).await {
                                                            Ok(val) => {
                                                                let close_result: JsSimpleResult = serde_wasm_bindgen::from_value(val).unwrap_or_else(|e| JsSimpleResult{ error: Some(format!("Deserialize error: {:?}", e))});
                                                                if let Some(err_msg) = close_result.error {
                                                                    error!("js_close_file failed for {}: {}", data_writer_id, err_msg);
                                                                    FsWindow::update_download_state_error(data_active_downloads, data_task_id, err_msg);
                                                                } else {
                                                                    // Status updated by progress task on GetEvent::Complete
                                                                }
                                                            }
                                                            Err(e_close) => {
                                                                 let err_msg = format!("js_close_file call failed: {:?}", e_close);
                                                                 error!("{}", err_msg);
                                                                 FsWindow::update_download_state_error(data_active_downloads, data_task_id, err_msg);
                                                            }
                                                        }
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });

                                // --- Spawn progress processing task ---
                                let prog_active_downloads = Arc::clone(&active_downloads_clone);
                                let prog_task_id = task_id;
                                let prog_egui_ctx = egui_ctx.clone();
                                let prog_stop_rx = stop_rx_for_picker_failure.clone();
                                spawn_local(async move {
                                    let mut prog_stop_rx_clone = (*prog_stop_rx).clone();
                                     loop {
                                        tokio::select! {
                                            changed_res = prog_stop_rx_clone.changed() => {
                                                 if changed_res.is_err() || *prog_stop_rx.borrow() {
                                                    info!("Progress task for {} stopping due to signal.", prog_task_id);
                                                    break;
                                                }
                                            }
                                            progress_event = progress_receiver.recv() => {
                                                match progress_event {
                                                    Some(Ok(TaskProgress::Get(get_event))) => {
                                                        match get_event {
                                                            GetEvent::Starting { total_chunks } => {
                                                                info!("Download Starting for {}: {} total chunks.", prog_task_id, total_chunks);
                                                            }
                                                            GetEvent::PadData { .. } => { }
                                                            GetEvent::Complete => {
                                                                info!("Download Complete event for {}", prog_task_id);
                                                                FsWindow::update_download_status(prog_active_downloads.clone(), prog_task_id, DownloadStatus::Completed, None);
                                                                prog_egui_ctx.request_repaint();
                                                                break; 
                                                            }
                                                            _ => {} 
                                                        }
                                                    }
                                                    Some(Err(e)) => {
                                                        let err_msg = format!("Progress stream error: {}", e);
                                                        error!("{}", err_msg);
                                                        FsWindow::update_download_state_error(prog_active_downloads.clone(), prog_task_id, err_msg);
                                                        prog_egui_ctx.request_repaint();
                                                        break; 
                                                    }
                                                    None => { 
                                                        info!("Progress stream ended for {}", prog_task_id);
                                                        // If data stream hasn't ended and caused completion, this might be premature.
                                                        // However, GetEvent::Complete should be the definitive signal.
                                                        break;
                                                    }
                                                    _ => {} 
                                                }
                                            }
                                        }
                                    }
                                });
                            } else {
                                let err_msg = "js_init_save_file returned no writerId and no error.".to_string();
                                error!("{}", err_msg);
                                let _ = stop_tx_for_picker_failure.send(true);
                                FsWindow::update_download_state_error(active_downloads_clone, task_id, err_msg);
                            }
                        }
                        Err(e) => { 
                            let err_msg = format!("js_init_save_file call failed: {:?}", e);
                            error!("{}", err_msg);
                            let _ = stop_tx_for_picker_failure.send(true);
                            FsWindow::update_download_state_error(active_downloads_clone, task_id, err_msg);
                        }
                    }
                }
                Err(e) => {
                    error!("start_streamed_get failed: {}", e);
                }
            }
            egui_ctx.request_repaint(); 
        });
    }


    // find_tab methods removed - use WindowSystem methods directly

    /// Add a new tab for a file - adds to the internal dock system
    fn add_file_tab(&mut self, file_details: KeyDetails) {
        log::info!("FsWindow: Creating new file viewer tab for: {}", file_details.key);

        // Check if a tab for this file already exists in the internal dock
        let tab_exists = self.internal_dock.iter_all_tabs().any(|(_, existing_tab)| {
            match existing_tab {
                FsInternalTab::FileViewer(file_tab) => file_tab.file.key == file_details.key,
                _ => false,
            }
        });

        if !tab_exists {
            // Create a new tab
            let file_tab = FileViewerTab::new(file_details.clone());
            let tab = FsInternalTab::FileViewer(file_tab);

            // Add directly to the internal dock system
            if self.internal_dock.iter_all_tabs().next().is_none() {
                // If the dock is empty, create a new surface
                self.internal_dock = egui_dock::DockState::new(vec![tab]);
            } else {
                // Add to the existing surface
                self.internal_dock.main_surface_mut().push_to_focused_leaf(tab);
            }

            log::info!("FsWindow: Successfully added tab to internal dock for: {}", file_details.key);

            // Start loading the file content asynchronously
            self.load_file_content(file_details);
        } else {
            log::info!("FsWindow: Tab for file {} already exists in internal dock", file_details.key);
        }
    }

    /// Load file content asynchronously and update the tab when complete
    fn load_file_content(&self, file_details: KeyDetails) {
        let key = file_details.key.clone();
        let is_public = file_details.is_public;

        log::info!("Starting async file loading for: {}", key);

        // Spawn async task to load file content
        wasm_bindgen_futures::spawn_local(async move {
            let ctx = crate::app::context::context();

            // Get the total file size for progress calculation
            let total_file_size = Some(file_details.total_size);
            let key_for_progress = key.clone();

            match ctx.get_file_for_viewing_with_progress(
                &key,
                is_public,
                move |downloaded_bytes, _total_bytes| {
                    // Update progress in the UI using the known total file size
                    if let Some(fs_window_ref) = get_main_fs_window() {
                        let mut fs_window = fs_window_ref.write().unwrap();

                        // Look for the file tab in the FsWindow's internal dock
                        for (_, internal_tab) in fs_window.internal_dock.iter_all_tabs_mut() {
                            if let FsInternalTab::FileViewer(file_tab) = internal_tab {
                                if file_tab.file.key == key_for_progress {
                                    file_tab.downloaded_bytes = downloaded_bytes;
                                    file_tab.total_bytes = total_file_size;

                                    // Calculate progress using the known total file size
                                    if let Some(total) = total_file_size {
                                        if total > 0 {
                                            file_tab.loading_progress = (downloaded_bytes as f32 / total as f32).min(1.0);
                                        } else {
                                            file_tab.loading_progress = 0.0;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            ).await {
                Ok(data) => {
                    log::info!("Successfully loaded file content for: {} ({} bytes)", key, data.len());

                    // Update the tab with the loaded content using the global FsWindow reference
                    if let Some(fs_window_ref) = get_main_fs_window() {
                        let mut fs_window = fs_window_ref.write().unwrap();

                        // Look for the file tab in the FsWindow's internal dock
                        for (_, internal_tab) in fs_window.internal_dock.iter_all_tabs_mut() {
                            if let FsInternalTab::FileViewer(file_tab) = internal_tab {
                                if file_tab.file.key == key {
                                    // Update the tab with loaded content
                                    file_tab.is_loading = false;

                                    // Determine file type and process content
                                    let file_type = super::components::multimedia::detect_file_type(&data, &key);
                                    file_tab.file_type = Some(file_type.clone());

                                    // Process content based on file type
                                    match file_type {
                                        super::components::multimedia::FileType::Text => {
                                            let content = String::from_utf8_lossy(&data).to_string();
                                            file_tab.content = content.clone();

                                            // Update FileContent
                                            if let Some(file_content) = &mut file_tab.file_content {
                                                file_content.file_type = file_type;
                                                file_content.raw_data = data;
                                                file_content.editable_content = Some(content);
                                                file_content.content_modified = false;
                                            }
                                        },
                                        super::components::multimedia::FileType::Code(lang) => {
                                            let content = String::from_utf8_lossy(&data).to_string();
                                            file_tab.content = content.clone();

                                            // Update FileContent
                                            if let Some(file_content) = &mut file_tab.file_content {
                                                file_content.file_type = super::components::multimedia::FileType::Code(lang);
                                                file_content.raw_data = data;
                                                file_content.editable_content = Some(content);
                                                file_content.content_modified = false;
                                            }
                                        },
                                        super::components::multimedia::FileType::Image => {
                                            // Store binary data for image processing
                                            file_tab.file_binary = Some(data.clone());

                                            // Create image texture for display
                                            // We need to get the egui context to create the texture
                                            // Since we're in an async context, we'll create the texture in the UI thread
                                            // For now, we'll store the data and create the texture when drawing

                                            // Update FileContent
                                            if let Some(file_content) = &mut file_tab.file_content {
                                                file_content.file_type = file_type;
                                                file_content.raw_data = data.clone();
                                                file_content.editable_content = None;
                                                file_content.content_modified = false;
                                            }
                                        },
                                        super::components::multimedia::FileType::Video => {
                                            // Store binary data for video processing
                                            file_tab.file_binary = Some(data.clone());

                                            // Update FileContent
                                            if let Some(file_content) = &mut file_tab.file_content {
                                                file_content.file_type = file_type;
                                                file_content.raw_data = data;
                                                file_content.editable_content = None;
                                                file_content.content_modified = false;
                                            }
                                        },
                                        super::components::multimedia::FileType::Other => {
                                            // Store binary data
                                            file_tab.file_binary = Some(data.clone());

                                            // Update FileContent
                                            if let Some(file_content) = &mut file_tab.file_content {
                                                file_content.file_type = file_type;
                                                file_content.raw_data = data;
                                                file_content.editable_content = None;
                                                file_content.content_modified = false;
                                            }
                                        },
                                    }

                                    log::info!("Updated file tab content for: {}", key);
                                    return; // Exit early since we found and updated the tab
                                }
                            }
                        }

                        log::warn!("Could not find file tab for key: {} in main FsWindow", key);
                    } else {
                        log::warn!("Main FsWindow reference not available for key: {}", key);
                    }
                },
                Err(e) => {
                    log::error!("Failed to load file content for {}: {}", key, e);

                    // Update the tab with error state using the global FsWindow reference
                    if let Some(fs_window_ref) = get_main_fs_window() {
                        let mut fs_window = fs_window_ref.write().unwrap();

                        // Look for the file tab in the FsWindow's internal dock
                        for (_, internal_tab) in fs_window.internal_dock.iter_all_tabs_mut() {
                            if let FsInternalTab::FileViewer(file_tab) = internal_tab {
                                if file_tab.file.key == key {
                                    file_tab.is_loading = false;
                                    file_tab.content = format!("Error loading file: {}", e);

                                    // Update FileContent with error
                                    if let Some(file_content) = &mut file_tab.file_content {
                                        file_content.editable_content = Some(file_tab.content.clone());
                                        file_content.raw_data = file_tab.content.as_bytes().to_vec();
                                    }
                                    return; // Exit early since we found and updated the tab
                                }
                            }
                        }

                        log::warn!("Could not find file tab for error update, key: {} in main FsWindow", key);
                    } else {
                        log::warn!("Main FsWindow reference not available for error update, key: {}", key);
                    }
                }
            }
        });
    }

    /// Add a new Put window tab to the internal dock system
    pub fn add_put_tab(&mut self) {
        log::info!("FsWindow: Creating new Put window tab");

        // Check if a Put tab already exists
        let tab_exists = self.internal_dock.iter_all_tabs().any(|(_, existing_tab)| {
            matches!(existing_tab, FsInternalTab::Put(_))
        });

        if !tab_exists {
            // Create a new Put window
            let put_window = PutWindow::new();
            let tab = FsInternalTab::Put(put_window);

            // Add to the internal dock system
            if self.internal_dock.iter_all_tabs().next().is_none() {
                // If the dock is empty, create a new surface
                self.internal_dock = egui_dock::DockState::new(vec![tab]);
            } else {
                // Add to the existing surface
                self.internal_dock.main_surface_mut().push_to_focused_leaf(tab);
            }

            log::info!("FsWindow: Successfully added Put tab to internal dock");
        } else {
            log::info!("FsWindow: Put tab already exists in internal dock");
        }
    }

    /// Add a new Stats window tab to the internal dock system
    pub fn add_stats_tab(&mut self) {
        log::info!("FsWindow: Creating new Stats window tab");

        // Check if a Stats tab already exists
        let tab_exists = self.internal_dock.iter_all_tabs().any(|(_, existing_tab)| {
            matches!(existing_tab, FsInternalTab::Stats(_))
        });

        if !tab_exists {
            // Create a new Stats window
            let stats_window = StatsWindow::new();
            let tab = FsInternalTab::Stats(stats_window);

            // Add to the internal dock system
            if self.internal_dock.iter_all_tabs().next().is_none() {
                // If the dock is empty, create a new surface
                self.internal_dock = egui_dock::DockState::new(vec![tab]);
            } else {
                // Add to the existing surface
                self.internal_dock.main_surface_mut().push_to_focused_leaf(tab);
            }

            log::info!("FsWindow: Successfully added Stats tab to internal dock");
        } else {
            log::info!("FsWindow: Stats tab already exists in internal dock");
        }
    }

    /// Check if a specific window type is currently open in the internal dock
    pub fn is_window_open(&self, window_name: &str) -> bool {
        self.internal_dock.iter_all_tabs().any(|(_, tab)| {
            match tab {
                FsInternalTab::Put(_) => window_name == "MutAnt Upload",
                FsInternalTab::Stats(_) => window_name == "MutAnt Stats",
                FsInternalTab::FileViewer(_) => false, // File viewers don't count for menu highlighting
            }
        })
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
        // Add minimal top padding
        ui.add_space(8.0);

        // Draw the tree with improved styling
        let scroll_response = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // Draw the continuous gradient bar FIRST (behind the content)
                let available_rect = ui.available_rect_before_wrap();
                let metadata_width = 100.0; // Balanced width for metadata area
                let fade_width = 40.0; // Balanced fade width for smooth gradient
                let fade_start = available_rect.width() - metadata_width - fade_width;
                let background_color = super::theme::MutantColors::BACKGROUND_MEDIUM;

                // Create a smooth continuous gradient bar that spans the entire available height
                let gradient_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(available_rect.left() + fade_start, available_rect.top()),
                    egui::Vec2::new(fade_width + metadata_width, available_rect.height())
                );

                // Create a darker version of the background color for the gradient target
                let darker_color = egui::Color32::from_rgb(
                    (background_color.r() as f32 * 0.4) as u8,
                    (background_color.g() as f32 * 0.4) as u8,
                    (background_color.b() as f32 * 0.4) as u8,
                );

                // Draw the gradient with more steps for smoother transition
                let steps = 30;
                let step_width = fade_width / steps as f32;

                for i in 0..steps {
                    let progress = i as f32 / (steps - 1) as f32; // 0.0 to 1.0

                    // Start with transparent (alpha = 0) and gradually increase alpha to show darker color
                    let alpha = (progress * 180.0) as u8; // Max alpha of 180 for subtle effect

                    let step_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(gradient_rect.left() + i as f32 * step_width, gradient_rect.top()),
                        egui::Vec2::new(step_width, gradient_rect.height())
                    );

                    let fade_color = egui::Color32::from_rgba_unmultiplied(
                        darker_color.r(),
                        darker_color.g(),
                        darker_color.b(),
                        alpha
                    );

                    ui.painter().rect_filled(step_rect, 0.0, fade_color);
                }

                // Draw a solid darker background for the metadata area with alpha blending
                let metadata_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(available_rect.right() - metadata_width, available_rect.top()),
                    egui::Vec2::new(metadata_width, available_rect.height())
                );
                let metadata_color = egui::Color32::from_rgba_unmultiplied(
                    darker_color.r(),
                    darker_color.g(),
                    darker_color.b(),
                    180 // Same alpha as the end of the gradient
                );
                ui.painter().rect_filled(metadata_rect, 0.0, metadata_color);

                // Add some top padding
                ui.add_space(12.0);

                // Get the currently selected path for highlighting
                let selected_path_ref = self.selected_path.as_deref();

                // Track clicked nodes to handle after the loop
                let mut view_details_clicked: Option<KeyDetails> = None;
                let mut download_details_clicked: Option<KeyDetails> = None;

                // Draw the root folder '/' as a proper collapsible folder
                let mut root_expanded = !self.root.children.is_empty(); // Default to expanded if there are children

                // Draw the root folder with refresh button positioned like a file's download button
                let row_response = ui.allocate_response(
                    egui::Vec2::new(ui.available_width(), 20.0),
                    egui::Sense::click()
                );

                let row_rect = row_response.rect;

                ui.horizontal(|ui| {
                    // Draw visual delimiter line for root folder
                    let line_color = super::theme::MutantColors::BORDER_LIGHT;
                    let line_x = 6.0; // Position line at the left edge
                    let line_start = ui.cursor().top();
                    let line_end = line_start + 20.0; // Height of one row

                    ui.painter().line_segment(
                        [egui::Pos2::new(line_x, line_start), egui::Pos2::new(line_x, line_end)],
                        egui::Stroke::new(1.0, line_color)
                    );

                    // Root folder as collapsible header
                    let icon = if root_expanded { "📂" } else { "📁" };
                    let text = RichText::new(format!("{} /", icon))
                        .size(12.0)  // Match other folder sizes
                        .color(super::theme::MutantColors::ACCENT_ORANGE);  // Special distinguishing color for folders

                    let header = egui::CollapsingHeader::new(text)
                        .id_salt(format!("mutant_fs_root_{}", self.window_id))
                        .default_open(root_expanded);

                    root_expanded = header.show(ui, |ui| {
                        // Sort children: directories first, then files
                        let mut sorted_children: Vec<_> = self.root.children.iter_mut().collect();
                        sorted_children.sort_by(|(_, a), (_, b)| {
                            match (a.is_dir(), b.is_dir()) {
                                (true, false) => std::cmp::Ordering::Less,    // Directories come before files
                                (false, true) => std::cmp::Ordering::Greater, // Files come after directories
                                _ => a.name.cmp(&b.name),                     // Sort alphabetically within each group
                            }
                        });

                        // Draw the sorted children with indentation level 1 (since they are children of the root '/')
                        for (_, child) in sorted_children {
                            let (view_details, download_details) = child.ui(ui, 1, selected_path_ref, &self.window_id);
                            if view_details.is_some() {
                                view_details_clicked = view_details;
                            }
                            if download_details.is_some() {
                                download_details_clicked = download_details;
                            }
                        }
                    }).header_response.clicked() || root_expanded;
                });

                // Draw refresh button at the same position as file download buttons
                let text_baseline_y = row_rect.top() + (row_rect.height() - 12.0) / 2.0;
                let button_width = 20.0;
                let current_x = row_rect.right() - 4.0; // Start with right padding

                let button_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(current_x - button_width, text_baseline_y),
                    egui::Vec2::new(button_width, 12.0)
                );

                // Draw refresh button manually
                let button_response = ui.allocate_rect(button_rect, egui::Sense::click());
                if button_response.clicked() {
                    // Trigger a refresh of the file list
                    let ctx = crate::app::context::context();
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = ctx.list_keys().await;
                    });
                }
                if button_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                // Draw the refresh icon at the exact text baseline
                let refresh_galley = ui.painter().layout_no_wrap(
                    "🔄".to_string(),
                    egui::FontId::new(12.0, egui::FontFamily::Proportional),
                    super::theme::MutantColors::TEXT_MUTED
                );
                ui.painter().galley(
                    egui::Pos2::new(current_x - button_width / 2.0 - refresh_galley.rect.width() / 2.0, text_baseline_y),
                    refresh_galley,
                    super::theme::MutantColors::TEXT_MUTED
                );



                // Add some bottom padding
                ui.add_space(8.0);

                (view_details_clicked, download_details_clicked)
            });



        // Handle the view clicked node outside the loop to avoid borrow issues
        if let Some(details) = scroll_response.inner.0 {
            // Update the selected path for highlighting in the tree
            self.selected_path = Some(details.key.clone());

            // Add a new tab for this file using the unified dock system
            // The add_file_tab method now automatically triggers async file loading
            self.add_file_tab(details);
        }

        // Handle the download click
        if let Some(details) = scroll_response.inner.1 {
            self.initiate_download(details, ui.ctx().clone());
        }
    }




}
