use eframe::egui::{self, Color32, Image, RichText, Sense, TextureHandle, TextureOptions, Ui};
use image;
use mime_guess::from_path;
use wasm_bindgen::JsCast;
use web_sys;
use base64::Engine;

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
    pub text_content: Option<String>,
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
            text_content: None,
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
                    self.text_content = Some(text);
                } else {
                    // If conversion fails, set as Other type
                    self.file_type = FileType::Other;
                }
            },
            FileType::Code(_) => {
                // Convert raw data to text for code display
                if let Ok(text) = String::from_utf8(self.raw_data.clone()) {
                    self.text_content = Some(text);
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

    match extension.to_lowercase().as_str() {
        "rs" => Some("rust".to_string()),
        "js" => Some("javascript".to_string()),
        "ts" => Some("typescript".to_string()),
        "py" => Some("python".to_string()),
        "java" => Some("java".to_string()),
        "c" => Some("c".to_string()),
        "cpp" | "cc" | "cxx" => Some("cpp".to_string()),
        "h" | "hpp" => Some("cpp".to_string()),
        "cs" => Some("csharp".to_string()),
        "go" => Some("go".to_string()),
        "rb" => Some("ruby".to_string()),
        "php" => Some("php".to_string()),
        "html" => Some("html".to_string()),
        "css" => Some("css".to_string()),
        "json" => Some("json".to_string()),
        "xml" => Some("xml".to_string()),
        "yaml" | "yml" => Some("yaml".to_string()),
        "md" => Some("markdown".to_string()),
        "sh" | "bash" => Some("bash".to_string()),
        "sql" => Some("sql".to_string()),
        "toml" => Some("toml".to_string()),
        _ => None,
    }
}

/// Draw a text viewer
pub fn draw_text_viewer(ui: &mut Ui, content: &str) {
    // For very large content, we'll use a more efficient approach
    if content.len() > 100_000 {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label("File is very large. Showing preview:");
            ui.separator();

            // Only show the first 50K characters to avoid performance issues
            let preview = if content.len() > 50_000 {
                &content[0..50_000]
            } else {
                content
            };

            ui.label(preview);

            if content.len() > 50_000 {
                ui.separator();
                ui.label("(File truncated due to size)");
            }
        });
    } else {
        // For smaller files, just use a monospace label for better performance
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.monospace(content);
        });
    }
}

/// Draw a code viewer with syntax highlighting
pub fn draw_code_viewer(ui: &mut Ui, content: &str, language: &str) {
    // For large files, skip syntax highlighting to avoid performance issues
    if content.len() > 50_000 {
        ui.label(RichText::new("File is too large for syntax highlighting. Showing plain text:").color(Color32::YELLOW));
        ui.separator();
        draw_text_viewer(ui, content);
        return;
    }

    // For markdown files specifically, use a simpler approach to avoid performance issues
    if language == "markdown" || language == "md" {
        ui.label(RichText::new("Markdown file:").color(Color32::GREEN));
        ui.separator();

        // Just use a monospace label for markdown files
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.monospace(content);
        });
        return;
    }

    // For other code files, use a simplified approach without full syntax highlighting
    ui.label(RichText::new(format!("Code file ({})", language)).color(Color32::LIGHT_BLUE));
    ui.separator();

    // Display the code with monospace font
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.monospace(content);
    });
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
    ui.label(RichText::new("Video Player").size(18.0));

    // Create a unique ID for the video element
    let video_id = format!("video_{:?}", ui.id());

    // Add a button to open the video in a new tab
    if ui.button("Open Video in Browser").clicked() {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url(video_url);
        }
    }

    ui.separator();

    // Create a frame for the video
    let _frame = egui::Frame::default()
        .fill(Color32::from_rgb(20, 20, 20))
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
    ui.colored_label(
        Color32::RED,
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
