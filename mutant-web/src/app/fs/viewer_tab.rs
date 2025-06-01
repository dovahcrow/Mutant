use mutant_protocol::KeyDetails;
use serde::{Deserialize, Serialize};
use eframe::egui;
use crate::app::components::multimedia;
use crate::app::theme;
use crate::app::context;
use crate::app::window_system;
use wasm_bindgen_futures::spawn_local;
use log;
use humansize::{format_size, BINARY}; // For humanize_size
use crate::app::fs_window::FsWindow; // For save_file -> get_window_mut FsWindow type

// Helper function to format file sizes (redefined locally)
fn humanize_size(size: usize) -> String {
    format_size(size, BINARY)
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

        let initial_content = "Loading file content...".to_string();
        let file_content_data = multimedia::FileContent {
            file_type: multimedia::FileType::Text,
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
            file_content: Some(file_content_data),
            loading_progress: 0.0,
            total_bytes: None,
            downloaded_bytes: 0,
            preferred_dock_area: None,
        }
    }

    /// Draw the file viewer tab
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let header_frame = egui::Frame::new()
            .fill(theme::MutantColors::BACKGROUND_MEDIUM)
            .stroke(egui::Stroke::new(1.0, theme::MutantColors::BORDER_DARK))
            .inner_margin(egui::Margin::same(8));

        header_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let file_name = std::path::Path::new(&self.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.file.key.clone());

                let (file_icon, icon_color) = {
                    let temp_node = crate::app::fs::tree::TreeNode::new_file(&file_name, self.file.clone());
                    temp_node.get_file_icon_and_color()
                };

                ui.label(egui::RichText::new(file_icon).size(16.0).color(icon_color));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&self.file.key).size(16.0).strong().color(theme::MutantColors::TEXT_PRIMARY));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.file_modified {
                        ui.label(egui::RichText::new("● Modified").size(11.0).color(theme::MutantColors::WARNING));
                        if ui.add(theme::primary_button("Save")).clicked() {
                            self.save_file(ui);
                        }
                    }
                    if self.file.is_public {
                        ui.label(egui::RichText::new("🌐 Public").size(11.0).color(theme::MutantColors::SUCCESS));
                    } else {
                        ui.label(egui::RichText::new("🔒 Private").size(11.0).color(theme::MutantColors::ACCENT_BLUE));
                    }
                });
            });

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                let file_size_str = humanize_size(self.file.total_size);
                ui.label(egui::RichText::new("📏").size(10.0).color(theme::MutantColors::ACCENT_CYAN));
                ui.label(egui::RichText::new(file_size_str).size(11.0).color(theme::MutantColors::TEXT_SECONDARY));
                ui.separator();
                ui.label(egui::RichText::new("🧩").size(10.0).color(theme::MutantColors::ACCENT_PURPLE));
                let pad_color = if self.file.confirmed_pads == self.file.pad_count {
                    theme::MutantColors::SUCCESS
                } else {
                    theme::MutantColors::WARNING
                };
                ui.label(egui::RichText::new(format!("{}/{} pads", self.file.confirmed_pads, self.file.pad_count)).size(11.0).color(pad_color));
                ui.separator();

                if let Some(file_type) = &self.file_type {
                    let (type_icon, type_color) = match file_type {
                        multimedia::FileType::Text => ("📄", theme::MutantColors::TEXT_SECONDARY),
                        multimedia::FileType::Code(_) => ("📝", theme::MutantColors::ACCENT_GREEN),
                        multimedia::FileType::Image => ("🖼️", theme::MutantColors::ACCENT_CYAN),
                        multimedia::FileType::Video => ("🎬", theme::MutantColors::ACCENT_PURPLE),
                        multimedia::FileType::Other => ("❓", theme::MutantColors::TEXT_MUTED),
                    };
                    let type_text = match file_type {
                        multimedia::FileType::Text => "Text".to_string(),
                        multimedia::FileType::Code(lang) => format!("Code ({})", lang),
                        multimedia::FileType::Image => "Image".to_string(),
                        multimedia::FileType::Video => "Video".to_string(),
                        multimedia::FileType::Other => "Unknown".to_string(),
                    };
                    ui.label(egui::RichText::new(type_icon).size(10.0).color(type_color));
                    ui.label(egui::RichText::new(type_text).size(11.0).color(type_color));
                }
                if let Some(addr) = &self.file.public_address {
                    ui.separator();
                    ui.label(egui::RichText::new("🔗").size(10.0).color(theme::MutantColors::ACCENT_BLUE));
                    ui.label(egui::RichText::new(format!("{}", &addr[..8])).size(10.0).color(theme::MutantColors::TEXT_MUTED));
                }
            });
        });

        ui.add_space(4.0);

        if self.is_loading {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading file content...");
                });
                if self.loading_progress > 0.0 || self.total_bytes.is_some() {
                    ui.add_space(4.0);
                    if let Some(total) = self.total_bytes {
                        ui.label(format!("Downloaded {} of {} ({}%)", humanize_size(self.downloaded_bytes), humanize_size(total), (self.loading_progress * 100.0) as u32));
                    } else if self.downloaded_bytes > 0 {
                        ui.label(format!("Downloaded {}", humanize_size(self.downloaded_bytes)));
                    }
                    ui.add_space(2.0);
                    ui.add(egui::ProgressBar::new(self.loading_progress).show_percentage().animate(true));
                }
                // Fallback for legacy progress
                if self.loading_progress == 0.0 && self.total_bytes.is_none() {
                    let app_ctx = context::context();
                    let get_id = format!("get_{}", self.file.key);
                    if let Some(progress) = app_ctx.get_get_progress(&get_id) {
                        let progress_guard = progress.read().unwrap();
                        if let Some(op) = progress_guard.operation.get("get") {
                            if op.total_pads > 0 {
                                let progress_value = op.nb_reserved as f32 / op.total_pads as f32;
                                ui.add_space(4.0);
                                ui.label(format!("Downloaded {} of {} pads", op.nb_reserved, op.total_pads));
                                ui.add_space(4.0);
                                ui.add(egui::ProgressBar::new(progress_value).show_percentage().animate(true));
                            }
                        }
                    }
                }
            });
            return;
        }

        if self.file_content.is_none() {
            log::warn!("FileContent not initialized for tab: {}", self.file.key);
        }

        if let Some(file_type) = &self.file_type {
            match file_type {
                multimedia::FileType::Text => {
                    if let Some(fc) = &mut self.file_content {
                        multimedia::draw_text_viewer(ui, fc);
                        if fc.content_modified {
                            if let Some(text) = &fc.editable_content { self.content = text.clone(); }
                            self.file_modified = true; fc.content_modified = false;
                        }
                    } else { ui.label("Error: File content not available"); }
                },
                multimedia::FileType::Code(lang) => {
                     if let Some(fc) = &mut self.file_content {
                        multimedia::draw_code_viewer(ui, fc, lang);
                        if fc.content_modified {
                           if let Some(text) = &fc.editable_content { self.content = text.clone(); }
                            self.file_modified = true; fc.content_modified = false;
                        }
                    } else { ui.label("Error: File content not available"); }
                },
                multimedia::FileType::Image => {
                    if self.image_texture.is_none() && self.file_binary.is_some() {
                        if let Some(binary_data) = &self.file_binary {
                            self.image_texture = multimedia::load_image(ui.ctx(), binary_data);
                        }
                    }
                    if let Some(texture) = &self.image_texture {
                        multimedia::draw_image_viewer(ui, texture);
                    } else if self.file_binary.is_some() {
                        ui.label("Error: Failed to create image texture");
                    } else { ui.label("Loading image..."); }
                },
                multimedia::FileType::Video => {
                    if let Some(url) = &self.video_url {
                        multimedia::draw_video_player(ui, url);
                    } else { ui.label("Error: Video URL not available"); }
                },
                multimedia::FileType::Other => {
                    multimedia::draw_unsupported_file(ui);
                },
            }
        } else {
            if let Some(fc) = &mut self.file_content {
                multimedia::draw_text_viewer(ui, fc);
                 if fc.content_modified {
                    if let Some(text) = &fc.editable_content { self.content = text.clone(); }
                    self.file_modified = true; fc.content_modified = false;
                }
            } else { ui.label("No file content available"); }
        }
    }

    /// Save the file content
    pub fn save_file(&mut self, ui: &mut egui::Ui) {
        let key_clone = self.file.key.clone();
        let is_public_clone = self.file.is_public;
        let content_clone = self.content.clone();
        self.is_loading = true;
        self.file_modified = false;
        let window_id_clone = ui.id(); // Clone window_id for async task

        spawn_local(async move {
            let app_ctx = context::context();
            let data_bytes = content_clone.into_bytes();
            let file_name_str = std::path::Path::new(&key_clone)
                .file_name().map_or_else(|| key_clone.clone(), |f| f.to_string_lossy().into_owned());

            match app_ctx.put(
                &key_clone, data_bytes, &file_name_str,
                mutant_protocol::StorageMode::Heaviest,
                is_public_clone, false, None,
            ).await {
                Ok(_) => {
                    let mut ws = window_system::window_system_mut();
                    if let Some(tab) = ws.find_file_viewer_tab_mut(&key_clone) {
                        tab.is_loading = false;
                    }
                    let _ = app_ctx.list_keys().await; // Refresh keys
                    if let Some(fs_win) = ws.get_window_mut::<FsWindow>(window_id_clone) {
                        fs_win.build_tree(); // Assuming FsWindow has build_tree
                    }
                },
                Err(e) => {
                    let mut ws = window_system::window_system_mut();
                    if let Some(tab) = ws.find_file_viewer_tab_mut(&key_clone) {
                        tab.is_loading = false;
                        tab.file_modified = true;
                        tab.content = format!("Error saving file: {}", e);
                    }
                }
            }
        });
    }
}
