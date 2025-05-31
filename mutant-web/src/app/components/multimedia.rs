use eframe::egui::{self, Image, RichText, Sense, TextureHandle, TextureOptions, Ui};
use egui_extras::syntax_highlighting::{self, CodeTheme};
use image;
use mime_guess::from_path;
use wasm_bindgen::JsCast;
use web_sys;
use base64::Engine;
use std::sync::Arc;

/// Enum representing different types of files
#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    Text,
    Image,
    Code(String), // String represents the language
    Video,
    Other,
}

/// Struct to hold file content in different formats
#[derive(Clone)]
pub struct FileContent {
    pub file_type: FileType,
    pub raw_data: Vec<u8>,
    pub editable_content: Option<String>,  // Separate field for edited content
    pub content_modified: bool,            // Flag to track if content was modified
    pub image_texture: Option<TextureHandle>,
    pub video_url: Option<String>,
}

impl FileContent {
    pub fn new(raw_data: Vec<u8>, file_path: &str) -> Self {
        let file_type = detect_file_type(&raw_data, file_path);

        // Initialize with raw data and detected type
        let mut content = Self {
            file_type,
            raw_data,
            editable_content: None,
            content_modified: false,
            image_texture: None,
            video_url: None,
        };

        // Process content based on file type
        content.process_content();

        content
    }

    fn process_content(&mut self) {
        match &self.file_type {
            FileType::Text => {
                // Try to convert raw data to text
                if let Ok(text) = String::from_utf8(self.raw_data.clone()) {
                    self.editable_content = Some(text); // Initialize editable content
                } else {
                    // If conversion fails, set as Other type
                    self.file_type = FileType::Other;
                }
            },
            FileType::Code(_) => {
                // Convert raw data to text for code display
                if let Ok(text) = String::from_utf8(self.raw_data.clone()) {
                    self.editable_content = Some(text); // Initialize editable content
                } else {
                    // If conversion fails, set as Other type
                    self.file_type = FileType::Other;
                }
            },
            _ => {
                // Other types are handled when rendering
            }
        }
    }

    /// Create a data URL for binary content (used for images and videos)
    pub fn create_data_url(&self, mime_type: &str) -> String {
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&self.raw_data);
        format!("data:{};base64,{}", mime_type, base64_data)
    }
}

/// Detect the type of file based on content and extension
pub fn detect_file_type(data: &[u8], file_path: &str) -> FileType {
    // First try to detect by MIME type from extension
    let mime = from_path(file_path).first_or_octet_stream();
    let mime_type = mime.essence_str();

    // Check for common MIME types
    if mime_type.starts_with("text/") {
        // For text files, check if it's code
        if let Some(lang) = get_language_from_extension(file_path) {
            return FileType::Code(lang);
        }
        return FileType::Text;
    } else if mime_type.starts_with("image/") {
        return FileType::Image;
    } else if mime_type.starts_with("video/") {
        return FileType::Video;
    }

    // If MIME detection fails, try to detect by content
    if let Ok(_) = String::from_utf8(data[0..std::cmp::min(1024, data.len())].to_vec()) {
        // If the first 1KB is valid UTF-8, it's probably text
        // Check if it's code
        if let Some(lang) = get_language_from_extension(file_path) {
            return FileType::Code(lang);
        }
        return FileType::Text;
    }

    // Check for image magic numbers
    if data.len() >= 4 {
        // PNG signature
        if &data[0..8] == &[137, 80, 78, 71, 13, 10, 26, 10] {
            return FileType::Image;
        }
        // JPEG signature
        if &data[0..3] == &[255, 216, 255] {
            return FileType::Image;
        }
        // GIF signature
        if &data[0..6] == &[71, 73, 70, 56, 57, 97] || &data[0..6] == &[71, 73, 70, 56, 55, 97] {
            return FileType::Image;
        }
    }

    // Default to Other if we can't determine the type
    FileType::Other
}

/// Get the programming language from a file extension
fn get_language_from_extension(file_path: &str) -> Option<String> {
    let extension = file_path.split('.').last()?;

    Some(extension.to_lowercase())
    //     "rs" => Some("rust".to_string()),
    //     "js" => Some("javascript".to_string()),
    //     "ts" => Some("typescript".to_string()),
    //     "py" => Some("python".to_string()),
    //     "java" => Some("java".to_string()),
    //     "c" => Some("c".to_string()),
    //     "cpp" | "cc" | "cxx" => Some("cpp".to_string()),
    //     "h" | "hpp" => Some("cpp".to_string()),
    //     "cs" => Some("csharp".to_string()),
    //     "go" => Some("go".to_string()),
    //     "rb" => Some("ruby".to_string()),
    //     "php" => Some("php".to_string()),
    //     "html" => Some("html".to_string()),
    //     "css" => Some("css".to_string()),
    //     "json" => Some("json".to_string()),
    //     "xml" => Some("xml".to_string()),
    //     "yaml" | "yml" => Some("yaml".to_string()),
    //     "md" => Some("markdown".to_string()),
    //     "sh" | "bash" => Some("bash".to_string()),
    //     "sql" => Some("sql".to_string()),
    //     "toml" => Some("toml".to_string()),
    //     _ => None,
    // }
}

// Get or create a MutAnt-themed code theme that respects the current context
fn get_code_theme(ui: &egui::Ui) -> Arc<CodeTheme> {
    // Create a custom dark theme that matches our MutAnt theme colors
    // We don't use static storage anymore to ensure it respects theme updates
    let theme = CodeTheme::dark(0.9); // Higher contrast for better readability

    // Store in context memory for this frame
    let theme_clone = theme.clone();
    theme_clone.store_in_memory(ui.ctx());
    Arc::new(theme)
}

/// Draw a text viewer with syntax highlighting
pub fn draw_text_viewer(ui: &mut Ui, file_content: &mut FileContent) {
    // For all files, use our optimized code editor with plain text syntax
    // The optimized version can handle large files efficiently
    draw_code_editor(ui, file_content, "txt");
}

/// Draw a code editor with syntax highlighting
fn draw_code_editor(ui: &mut Ui, file_content: &mut FileContent, language: &str) {
    let theme = get_code_theme(ui);

    // Get a mutable reference to the editable content
    let content = match file_content.editable_content.as_mut() {
        Some(content) => content,
        None => {
            ui.label("Error: No editable content available");
            return;
        }
    };

    // Don't override the style - let it inherit from the root theme

    // Use a more efficient scroll area that only renders visible content
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            // Use a more efficient layouter with caching
            // This is the key to performance improvement
            let mut layouter = move |ui: &egui::Ui, string: &str, wrap_width: f32| {
                // Create a new layout job with syntax highlighting
                let mut job = syntax_highlighting::highlight(
                    ui.ctx(),
                    ui.style(),
                    &theme,
                    string,
                    language,
                );

                job.wrap.max_width = wrap_width;

                // Return the layout job
                ui.fonts(|f| f.layout_job(job))
            };

            // Add the TextEdit with the optimized layouter
            let response = ui.add(
                egui::TextEdit::multiline(content)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .interactive(true) // Make it editable
            );

            // Check if the content has changed
            if response.changed() {
                file_content.content_modified = true;
            }
        });
}

/// Draw a code viewer with syntax highlighting
pub fn draw_code_viewer(ui: &mut Ui, file_content: &mut FileContent, language: &str) {
    // Display file type information
    ui.label(format!("Code file ({})", language));
    ui.separator();

    // Use our optimized code editor with the appropriate language
    // The optimized version can handle large files efficiently
    draw_code_editor(ui, file_content, language);
}

/// Draw an image viewer
pub fn draw_image_viewer(ui: &mut Ui, texture: &TextureHandle) {
    egui::ScrollArea::both().show(ui, |ui| {
        // Display the image
        ui.add(Image::new(texture).sense(Sense::click()));
    });
}

/// Draw a video player
pub fn draw_video_player(ui: &mut Ui, video_url: &str) {
    use crate::app::theme::MutantColors;

    ui.label(RichText::new("Video Player").size(18.0).color(MutantColors::TEXT_PRIMARY));

    // Create a unique ID for the video element
    let video_id = format!("video_{:?}", ui.id());

    // Add a button to open the video in a new tab with MutAnt theme
    if ui.add(crate::app::theme::secondary_button("Open Video in Browser")).clicked() {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url(video_url);
        }
    }

    ui.separator();

    // Create a frame for the video with MutAnt theme colors
    let _frame = egui::Frame::default()
        .fill(MutantColors::BACKGROUND_DARK)
        .show(ui, |ui| {
            // Reserve space for the video
            let desired_size = egui::vec2(640.0, 360.0);
            let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

            // Create or update the video element
            if let Some(window) = web_sys::window() {
                if let Some(document) = window.document() {
                    // Check if the video element already exists
                    let video_element = match document.get_element_by_id(&video_id) {
                        Some(element) => element,
                        None => {
                            // Create a new video element
                            let element = document.create_element("video").unwrap();
                            element.set_id(&video_id);

                            // Set video attributes
                            element.set_attribute("controls", "true").unwrap();
                            element.set_attribute("style", "position: absolute; z-index: 1;").unwrap();

                            // Create a source element
                            let source = document.create_element("source").unwrap();
                            source.set_attribute("src", video_url).unwrap();

                            // Append source to video
                            element.append_child(&source).unwrap();

                            // Append video to document body
                            document.body().unwrap().append_child(&element).unwrap();

                            element
                        }
                    };

                    // Position the video element over the allocated space
                    let html_element = video_element.dyn_into::<web_sys::HtmlElement>().unwrap();
                    html_element.style().set_property("position", "absolute").unwrap();
                    html_element.style().set_property("left", &format!("{}px", rect.min.x)).unwrap();
                    html_element.style().set_property("top", &format!("{}px", rect.min.y)).unwrap();
                    html_element.style().set_property("width", &format!("{}px", rect.width())).unwrap();
                    html_element.style().set_property("height", &format!("{}px", rect.height())).unwrap();
                }
            }
        });
}

/// Draw an error message for unsupported file types
pub fn draw_unsupported_file(ui: &mut Ui) {
    use crate::app::theme::MutantColors;

    ui.colored_label(
        MutantColors::ERROR,
        "This file type is not supported for viewing. You can download the file instead."
    );
}

/// Load an image from raw data
pub fn load_image(ctx: &egui::Context, data: &[u8]) -> Option<TextureHandle> {
    // Try to load the image
    let image = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(_) => return None,
    };

    // Convert to RGBA
    let image_rgba = image.to_rgba8();
    let size = [image_rgba.width() as _, image_rgba.height() as _];
    let image_data = egui::ColorImage::from_rgba_unmultiplied(size, &image_rgba);

    // Create texture
    Some(ctx.load_texture(
        "image_viewer_texture",
        image_data,
        TextureOptions::default(),
    ))
}
