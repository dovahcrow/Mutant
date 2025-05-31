use std::sync::{Arc, RwLock, Mutex}; // Added Mutex
use std::collections::BTreeMap;

use eframe::egui::{self, Color32, RichText};

use humansize::{format_size, BINARY};
use mutant_protocol::{KeyDetails, TaskProgress, GetEvent, TaskId}; // Added TaskProgress, GetEvent, TaskId
use serde::{Deserialize, Serialize};
use base64::Engine;

use super::components::multimedia;
use super::Window;
use super::theme::secondary_button;
use super::window_system::generate_unique_dock_area_id;
use crate::utils::download_utils::{self, JsFileHandleResult, JsSimpleResult};
use js_sys::Uint8Array;
use wasm_bindgen_futures::spawn_local;
use log::{info, error}; // Ensure log levels are imported
use serde_wasm_bindgen;


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
        // Use a very small fixed indentation to avoid exponential growth
        let indent_per_level = 2.0;
        let total_indent = indent_per_level * (indent_level as f32);

        let mut view_clicked_details = None;
        let mut download_clicked_details = None;

        ui.horizontal(|ui| {
            // Apply the base indentation
            ui.add_space(total_indent);

            if self.is_dir() {
                // Directory node
                let icon = if self.expanded { "📂" } else { "📁" };
                let text = format!("{} {}/", icon, self.name); // Add '/' at the end of folder names

                let header = egui::CollapsingHeader::new(text)
                    .id_salt(format!("mutant_fs_{}dir_{}", window_id, self.path)) // Use window ID + path for unique ID
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

                let icon = "📄";
                let details = self.key_details.as_ref().unwrap();

                // Check if this file is selected
                let is_selected = selected_path.map_or(false, |path| path == &details.key);

                // Style the text based on selection and public status
                let text_display = if details.is_public {
                    let text = RichText::new(format!("{} {} (public)", icon, self.name))
                        .color(Color32::from_rgb(0, 128, 0));
                    text
                } else {
                    RichText::new(format!("{} {}", icon, self.name))
                };

                // Make the file node clickable for viewing
                if ui.selectable_label(is_selected, text_display).clicked() {
                    view_clicked_details = Some(details.clone());
                }
                
                // Add a download button
                if ui.add(secondary_button("💾")).on_hover_text("Download file").clicked() {
                    download_clicked_details = Some(details.clone());
                }
            }
        });

        (view_clicked_details, download_clicked_details)
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
}

impl FileViewerTab {
    /// Create a new file viewer tab
    pub fn new(file: KeyDetails) -> Self {
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

        Self {
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
        }
    }

    /// Draw the file viewer tab
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // Show file details at the top
        ui.heading(&self.file.key);

        // Store file details we need for display
        let file_size = humanize_size(self.file.total_size);
        let pad_count = self.file.pad_count;
        let confirmed_pads = self.file.confirmed_pads;
        let is_public = self.file.is_public;
        let public_address = self.file.public_address.clone();

        // Store current state
        let file_type = self.file_type.clone();
        let file_modified = self.file_modified;

        ui.horizontal(|ui| {
            ui.label(format!("Size: {}", file_size));
            ui.separator();
            ui.label(format!("Pads: {}/{}", confirmed_pads, pad_count));
            ui.separator();
            if is_public {
                ui.label(RichText::new("Public").color(crate::app::theme::MutantColors::SUCCESS));
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
                ui.label(RichText::new("Modified").color(crate::app::theme::MutantColors::WARNING));

                if ui.add(crate::app::theme::primary_button("Save")).clicked() {
                    // Save the file
                    self.save_file(ui);
                }
            }
        });

        ui.separator();

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

// FileViewerTabViewer is no longer needed - using unified dock system

/// The filesystem tree window
#[derive(Clone, Serialize, Deserialize)]
pub struct FsWindow {
    keys: Arc<RwLock<Vec<KeyDetails>>>,
    root: TreeNode,
    /// Path of the selected file (for highlighting in the tree)
    selected_path: Option<String>,
    // Removed internal dock system - now using unified dock system
    #[serde(skip)] // active_downloads should not be serialized
    active_downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    /// Unique identifier for this window instance to avoid widget ID conflicts
    #[serde(skip)]
    window_id: String,
    /// Unique dock area ID for the file viewer
    #[serde(skip)]
    dock_area_id: String,
}

impl Default for FsWindow {
    fn default() -> Self {
        Self {
            keys: crate::app::context::context().get_key_cache(),
            root: TreeNode::default(),
            selected_path: None,
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

        // Use a horizontal layout with two panels
        egui::SidePanel::left(format!("mutant_fs_tree_panel_{}", self.window_id))
            .resizable(true)
            .min_width(200.0)
            .default_width(300.0)
            .show_inside(ui, |ui| {
                self.draw_tree(ui);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Show instructions for using the unified dock system
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("File Viewer");
                    ui.add_space(10.0);
                    ui.label("Click on a file in the tree to open it in a new tab.");
                    ui.label("File tabs will appear in the main dock area and can be moved freely.");
                });
            });
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

    /// Add a new tab for a file - now uses the unified dock system
    fn add_file_tab(&mut self, file_details: KeyDetails) {
        // Create a new tab
        let tab = FileViewerTab::new(file_details.clone());

        // Use the unified dock system to add the tab
        crate::app::window_system::new_file_viewer_tab(tab);
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
            let mut view_details_clicked: Option<KeyDetails> = None;
            let mut download_details_clicked: Option<KeyDetails> = None;


            // Draw the sorted children with indentation and check if any file was clicked
            for (_, child) in sorted_children {
                let (view_details, download_details) = child.ui(ui, 1, selected_path_ref, &self.window_id); // Start with indent level 1 since we're inside root
                if view_details.is_some() {
                    view_details_clicked = view_details;
                }
                if download_details.is_some() {
                    download_details_clicked = download_details;
                }
            }

            // Handle the view clicked node outside the loop to avoid borrow issues
            if let Some(details) = view_details_clicked {
                // Update the selected path for highlighting in the tree
                self.selected_path = Some(details.key.clone());

                // Add a new tab for this file using the unified dock system
                self.add_file_tab(details);

                // TODO: Add async file loading logic here
                // For now, the tab is created empty and will be loaded later
            }
            
            // Handle the download click
            if let Some(details) = download_details_clicked {
                self.initiate_download(details, ui.ctx().clone());
            }
        });
    }


}
