use serde::{Deserialize, Serialize};
use eframe::egui;
use crate::app::{Window, context::context};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

// Global state for storing user contact info responses
lazy_static::lazy_static! {
    static ref USER_CONTACT_RESPONSES: Arc<Mutex<HashMap<String, UserContactInfo>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref CONTENT_LIST_RESPONSES: Arc<Mutex<HashMap<String, Vec<ContentItem>>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// The Colony window for managing contacts and discovering content
#[derive(Clone, Serialize, Deserialize)]
pub struct ColonyWindow {
    /// List of contacts (pod addresses)
    contacts: Vec<Contact>,
    /// Available content from all contacts
    content_list: Vec<ContentItem>,
    /// Search query for filtering content
    search_query: String,
    /// New contact input fields
    new_contact_address: String,
    new_contact_name: String,
    /// UI state
    #[serde(skip)]
    is_syncing: bool,
    #[serde(skip)]
    last_sync_status: Option<String>,
    /// User's own contact information
    #[serde(skip)]
    user_contact_info: Option<UserContactInfo>,
    #[serde(skip)]
    is_loading_user_contact: bool,
    /// Flag to trigger contact info reload
    #[serde(skip)]
    should_load_contact_info: bool,
    /// Content list loading state
    #[serde(skip)]
    is_loading_content: bool,
    /// Flag to trigger content list reload
    #[serde(skip)]
    should_load_content: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Contact {
    pub pod_address: String,
    pub name: Option<String>,
    pub last_synced: Option<String>,
}

#[derive(Clone)]
pub struct UserContactInfo {
    pub contact_address: String,
    pub contact_type: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ContentItem {
    pub title: String,
    pub description: Option<String>,
    pub content_type: String,
    pub address: String,
    pub source_contact: String,
    pub size: Option<u64>,
    pub date_created: Option<String>,
}

impl Default for ColonyWindow {
    fn default() -> Self {
        Self {
            contacts: Vec::new(),
            content_list: Vec::new(),
            search_query: String::new(),
            new_contact_address: String::new(),
            new_contact_name: String::new(),
            is_syncing: false,
            last_sync_status: None,
            user_contact_info: None,
            is_loading_user_contact: false,
            should_load_contact_info: true,
            is_loading_content: false,
            should_load_content: true,
        }
    }
}

impl Window for ColonyWindow {
    fn name(&self) -> String {
        "Colony".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        // Auto-load user contact info on first draw if not already loaded/loading
        if self.should_load_contact_info && !self.is_loading_user_contact {
            self.load_user_contact_info();
            self.should_load_contact_info = false;
        }

        // Auto-load content list on first draw if not already loaded/loading
        if self.should_load_content && !self.is_loading_content {
            self.load_content_list();
            self.should_load_content = false;
        }

        // Check for completed user contact info response
        if self.is_loading_user_contact {
            if let Ok(responses) = USER_CONTACT_RESPONSES.lock() {
                if let Some(contact_info) = responses.get("user_contact") {
                    self.user_contact_info = Some(contact_info.clone());
                    self.is_loading_user_contact = false;
                    // Remove the response from the global state
                    drop(responses);
                    if let Ok(mut responses) = USER_CONTACT_RESPONSES.lock() {
                        responses.remove("user_contact");
                    }
                } else if responses.contains_key("user_contact_error") {
                    // Handle error case
                    self.is_loading_user_contact = false;
                    drop(responses);
                    if let Ok(mut responses) = USER_CONTACT_RESPONSES.lock() {
                        responses.remove("user_contact_error");
                    }
                }
            }
        }

        // Check for completed content list response
        if self.is_loading_content {
            if let Ok(responses) = CONTENT_LIST_RESPONSES.lock() {
                if let Some(content_list) = responses.get("content_list") {
                    self.content_list = content_list.clone();
                    self.is_loading_content = false;
                    // Remove the response from the global state
                    drop(responses);
                    if let Ok(mut responses) = CONTENT_LIST_RESPONSES.lock() {
                        responses.remove("content_list");
                    }
                } else if responses.contains_key("content_list_error") {
                    // Handle error case
                    self.is_loading_content = false;
                    drop(responses);
                    if let Ok(mut responses) = CONTENT_LIST_RESPONSES.lock() {
                        responses.remove("content_list_error");
                    }
                }
            }
        }
        // Use a vertical layout for the main content
        ui.vertical(|ui| {
            // Header section
            ui.horizontal(|ui| {
                ui.heading("Colony - Content Discovery");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Sync button
                    let sync_button = ui.add_enabled(
                        !self.is_syncing,
                        egui::Button::new(if self.is_syncing { "Syncing..." } else { "🔄 Sync All" })
                            .fill(super::theme::MutantColors::ACCENT_ORANGE)
                    );
                    
                    if sync_button.clicked() {
                        self.sync_all_contacts();
                    }
                });
            });

            ui.separator();

            // User's own contact information section
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Your Contact Address")
                            .strong()
                            .color(super::theme::MutantColors::ACCENT_ORANGE)
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🔄 Refresh").clicked() {
                            self.user_contact_info = None;
                            self.is_loading_user_contact = false;
                            self.should_load_contact_info = true;
                        }
                    });
                });

                if self.is_loading_user_contact {
                    ui.label("Loading your contact information...");
                } else if let Some(user_info) = &self.user_contact_info {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Share this address with friends so they can add you as a contact:")
                                    .size(11.0)
                                    .color(super::theme::MutantColors::TEXT_SECONDARY)
                            );

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&user_info.contact_address)
                                        .strong()
                                        .color(super::theme::MutantColors::ACCENT_BLUE)
                                );

                                if ui.button("📋 Copy").clicked() {
                                    ui.ctx().copy_text(user_info.contact_address.clone());
                                }
                            });

                            if let Some(display_name) = &user_info.display_name {
                                ui.label(
                                    egui::RichText::new(format!("Display: {}", display_name))
                                        .size(10.0)
                                        .color(super::theme::MutantColors::TEXT_MUTED)
                                );
                            }

                            ui.label(
                                egui::RichText::new(format!("Type: {}", user_info.contact_type))
                                    .size(10.0)
                                    .color(super::theme::MutantColors::TEXT_MUTED)
                            );
                        });
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Click 'Refresh' to load your contact information");
                        if ui.button("Load Now").clicked() {
                            self.user_contact_info = None;
                            self.is_loading_user_contact = false;
                            self.should_load_contact_info = true;
                        }
                    });
                }
            });

            ui.separator();

            // Two-column layout
            ui.horizontal(|ui| {
                // Left column - Contacts management
                ui.vertical(|ui| {
                    ui.set_min_width(300.0);
                    ui.set_max_width(300.0);
                    
                    ui.heading("Contacts");
                    
                    // Add new contact section
                    ui.group(|ui| {
                        ui.label("Add New Contact:");
                        
                        ui.horizontal(|ui| {
                            ui.label("Pod Address:");
                            ui.text_edit_singleline(&mut self.new_contact_address);
                        });
                        
                        ui.horizontal(|ui| {
                            ui.label("Name (optional):");
                            ui.text_edit_singleline(&mut self.new_contact_name);
                        });
                        
                        ui.horizontal(|ui| {
                            if ui.button("Add Contact").clicked() {
                                self.add_contact();
                            }
                            
                            if ui.button("Clear").clicked() {
                                self.new_contact_address.clear();
                                self.new_contact_name.clear();
                            }
                        });
                    });

                    ui.separator();

                    // Contacts list
                    ui.label(format!("Contacts ({})", self.contacts.len()));

                    ui.push_id("contacts_scroll", |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .show(ui, |ui| {
                                for (index, contact) in self.contacts.iter().enumerate() {
                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                if let Some(name) = &contact.name {
                                                    ui.label(
                                                        egui::RichText::new(name)
                                                            .strong()
                                                            .color(super::theme::MutantColors::ACCENT_ORANGE)
                                                    );
                                                }
                                                ui.label(
                                                    egui::RichText::new(&contact.pod_address)
                                                        .size(10.0)
                                                        .color(super::theme::MutantColors::TEXT_MUTED)
                                                );
                                                if let Some(last_synced) = &contact.last_synced {
                                                    ui.label(
                                                        egui::RichText::new(format!("Last synced: {}", last_synced))
                                                            .size(9.0)
                                                            .color(super::theme::MutantColors::TEXT_MUTED)
                                                    );
                                                }
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("🗑").clicked() {
                                                    // Mark for removal (we'll handle this after the loop)
                                                    // For now, just log it
                                                    log::info!("Remove contact at index {}", index);
                                                }
                                            });
                                        });
                                    });
                                }
                            });
                    });
                });

                ui.separator();

                // Right column - Content discovery
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Available Content");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Refresh button
                            let refresh_button = ui.add_enabled(
                                !self.is_loading_content,
                                egui::Button::new(if self.is_loading_content { "Loading..." } else { "🔄 Refresh" })
                                    .fill(super::theme::MutantColors::ACCENT_BLUE)
                            );

                            if refresh_button.clicked() {
                                self.load_content_list();
                            }
                        });
                    });

                    // Search bar
                    ui.horizontal(|ui| {
                        ui.label("Search:");
                        ui.text_edit_singleline(&mut self.search_query);
                        if ui.button("🔍").clicked() {
                            self.search_content();
                        }
                        if ui.button("Clear").clicked() {
                            self.search_query.clear();
                            self.load_content_list();
                        }
                    });

                    ui.separator();

                    // Content list
                    ui.label(format!("Content Items ({})", self.content_list.len()));

                    ui.push_id("content_scroll", |ui| {
                        egui::ScrollArea::vertical()
                            .show(ui, |ui| {
                                for content in &self.content_list {
                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&content.title)
                                                        .strong()
                                                        .color(super::theme::MutantColors::ACCENT_BLUE)
                                                );

                                                if let Some(description) = &content.description {
                                                    ui.label(
                                                        egui::RichText::new(description)
                                                            .size(11.0)
                                                            .color(super::theme::MutantColors::TEXT_SECONDARY)
                                                    );
                                                }

                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(&content.content_type)
                                                            .size(10.0)
                                                            .color(super::theme::MutantColors::ACCENT_GREEN)
                                                    );

                                                    ui.label(
                                                        egui::RichText::new(format!("by: {}", content.source_contact))
                                                            .size(10.0)
                                                            .color(super::theme::MutantColors::TEXT_MUTED)
                                                    );

                                                    if let Some(size) = content.size {
                                                        ui.label(
                                                            egui::RichText::new(Self::format_file_size(size))
                                                                .size(10.0)
                                                                .color(super::theme::MutantColors::TEXT_MUTED)
                                                        );
                                                    }

                                                    if let Some(date) = &content.date_created {
                                                        ui.label(
                                                            egui::RichText::new(Self::format_date(date))
                                                                .size(10.0)
                                                                .color(super::theme::MutantColors::TEXT_MUTED)
                                                        );
                                                    }
                                                });
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("📥 Download").clicked() {
                                                    self.download_content(&content.address);
                                                }
                                            });
                                        });
                                    });
                                }
                            });
                    });
                });
            });

            // Status bar at bottom
            if let Some(status) = &self.last_sync_status {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    ui.label(
                        egui::RichText::new(status)
                            .color(super::theme::MutantColors::SUCCESS)
                    );
                });
            }
        });
    }
}

impl ColonyWindow {
    /// Add a new contact
    fn add_contact(&mut self) {
        if !self.new_contact_address.trim().is_empty() {
            let contact = Contact {
                pod_address: self.new_contact_address.trim().to_string(),
                name: if self.new_contact_name.trim().is_empty() {
                    None
                } else {
                    Some(self.new_contact_name.trim().to_string())
                },
                last_synced: None,
            };

            // Add to local list
            self.contacts.push(contact.clone());

            // Send to daemon
            let ctx = context();
            let pod_address = contact.pod_address.clone();
            let contact_name = contact.name.clone();
            
            wasm_bindgen_futures::spawn_local(async move {
                match ctx.add_contact(&pod_address, contact_name).await {
                    Ok(_) => {
                        log::info!("Successfully added contact: {}", pod_address);
                    }
                    Err(e) => {
                        log::error!("Failed to add contact {}: {:?}", pod_address, e);
                    }
                }
            });

            // Clear input fields
            self.new_contact_address.clear();
            self.new_contact_name.clear();
        }
    }

    /// Sync all contacts to get latest content
    fn sync_all_contacts(&mut self) {
        if self.is_syncing {
            return;
        }

        self.is_syncing = true;
        self.last_sync_status = Some("Syncing contacts...".to_string());

        let ctx = context();
        wasm_bindgen_futures::spawn_local(async move {
            match ctx.sync_contacts().await {
                Ok(response) => {
                    log::info!("Sync completed: {} contacts synced", response.synced_count);
                    // TODO: Update UI state
                }
                Err(e) => {
                    log::error!("Sync failed: {:?}", e);
                    // TODO: Update UI state with error
                }
            }
        });
    }

    /// Search for content
    fn search_content(&mut self) {
        if self.search_query.trim().is_empty() {
            self.refresh_content_list();
            return;
        }

        let query = serde_json::json!({
            "type": "text",
            "text": self.search_query.trim(),
            "limit": 50
        });

        let ctx = context();
        wasm_bindgen_futures::spawn_local(async move {
            match ctx.search(query).await {
                Ok(response) => {
                    log::info!("Search completed: {:?}", response.results);
                    // TODO: Parse results and update content list
                }
                Err(e) => {
                    log::error!("Search failed: {:?}", e);
                }
            }
        });
    }

    /// Load the content list from the daemon
    fn load_content_list(&mut self) {
        if self.is_loading_content {
            return;
        }

        self.is_loading_content = true;

        let ctx = context();
        wasm_bindgen_futures::spawn_local(async move {
            match ctx.list_content().await {
                Ok(response) => {
                    log::info!("Content list loaded: {:?}", response.content);

                    // Parse the SPARQL results into ContentItem structures
                    let content_items = Self::parse_content_response(response.content);

                    // Store the response in global state for the UI to pick up
                    if let Ok(mut responses) = CONTENT_LIST_RESPONSES.lock() {
                        responses.insert("content_list".to_string(), content_items);
                    }
                }
                Err(e) => {
                    log::error!("Failed to load content list: {:?}", e);
                    // Store error state
                    if let Ok(mut responses) = CONTENT_LIST_RESPONSES.lock() {
                        responses.insert("content_list_error".to_string(), Vec::new());
                    }
                }
            }
        });
    }

    /// Refresh the content list (alias for load_content_list for backward compatibility)
    fn refresh_content_list(&mut self) {
        self.load_content_list();
    }

    /// Parse search query results into ContentItem structures
    fn parse_content_response(content: serde_json::Value) -> Vec<ContentItem> {
        let mut content_items = Vec::new();

        // Handle error responses
        if let Some(error) = content.get("error") {
            log::warn!("Search returned error: {}", error);
            return content_items;
        }

        // Only handle SPARQL results format (sparql_results -> results -> bindings)
        if let Some(sparql_results) = content.get("sparql_results") {
            if let Some(results) = sparql_results.get("results") {
                if let Some(bindings) = results.get("bindings") {
                    if let Some(bindings_array) = bindings.as_array() {
                        log::info!("Found {} SPARQL bindings to parse", bindings_array.len());
                        content_items.extend(Self::parse_sparql_bindings(bindings_array));
                    }
                }
            }
        } else {
            log::warn!("Expected SPARQL results format but got: {:?}", content);
        }

        log::info!("Parsed {} content items from response", content_items.len());
        content_items
    }

    /// Parse multiple SPARQL bindings into ContentItem structures
    fn parse_sparql_bindings(bindings_array: &[serde_json::Value]) -> Vec<ContentItem> {
        use std::collections::HashMap;

        // Group bindings by subject to reconstruct complete objects
        let mut subjects: HashMap<String, HashMap<String, String>> = HashMap::new();

        for binding in bindings_array {
            if let (Some(subject), Some(predicate), Some(object)) = (
                binding.get("subject").and_then(|s| s.get("value")).and_then(|v| v.as_str()),
                binding.get("predicate").and_then(|p| p.get("value")).and_then(|v| v.as_str()),
                binding.get("object").and_then(|o| o.get("value")).and_then(|v| v.as_str()),
            ) {
                subjects.entry(subject.to_string())
                    .or_insert_with(HashMap::new)
                    .insert(predicate.to_string(), object.to_string());
            }
        }

        let mut content_items = Vec::new();

        for (subject_uri, properties) in subjects {
            // Extract the address from the subject URI or url property
            let address = properties.get("http://schema.org/url")
                .or_else(|| properties.get("schema:url"))
                .cloned()
                .unwrap_or_else(|| {
                    if subject_uri.starts_with("ant://") {
                        subject_uri.replace("ant://", "")
                    } else {
                        subject_uri.clone()
                    }
                });

            // Extract Schema.org properties (try both formats: with and without schema: prefix)
            let title = properties.get("http://schema.org/name")
                .or_else(|| properties.get("schema:name"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    // If no name, create a title from the address
                    if address.len() > 16 {
                        format!("Content {}...{}", &address[0..8], &address[address.len()-8..])
                    } else {
                        format!("Content {}", address)
                    }
                });

            let description = properties.get("http://schema.org/description")
                .or_else(|| properties.get("schema:description"))
                .map(|s| s.to_string());

            let content_type = properties.get("http://schema.org/type")
                .or_else(|| properties.get("@type"))
                .map(|s| Self::format_content_type(s))
                .unwrap_or_else(|| "Content".to_string());

            let source_contact = properties.get("http://schema.org/author")
                .or_else(|| properties.get("schema:author"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let size = properties.get("http://schema.org/contentSize")
                .or_else(|| properties.get("schema:contentSize"))
                .and_then(|s| s.parse::<u64>().ok());

            let date_created = properties.get("http://schema.org/dateCreated")
                .or_else(|| properties.get("schema:dateCreated"))
                .map(|s| s.to_string());

            content_items.push(ContentItem {
                title,
                description,
                content_type,
                address,
                source_contact,
                size,
                date_created,
            });
        }

        log::info!("Converted {} subjects into content items", content_items.len());
        content_items
    }



    /// Format content type for display
    fn format_content_type(content_type: &str) -> String {
        // Convert Schema.org URIs to readable format
        if content_type.starts_with("http://schema.org/") {
            content_type.replace("http://schema.org/", "")
        } else if content_type.starts_with("https://schema.org/") {
            content_type.replace("https://schema.org/", "")
        } else {
            content_type.to_string()
        }
    }

    /// Download content by address and open in new viewer tab
    fn download_content(&self, address: &str) {
        log::info!("Downloading content from address: {}", address);

        // Create a KeyDetails for the public address
        let key_details = mutant_protocol::KeyDetails {
            key: address.to_string(),
            total_size: 0, // Unknown size for public content
            pad_count: 0,  // Unknown pad count
            confirmed_pads: 0, // Unknown confirmed pads
            is_public: true,
            public_address: Some(address.to_string()),
        };

        // Add the tab to the main fs window's internal dock system
        if let Some(fs_window_ref) = crate::app::fs::global::get_main_fs_window() {
            let mut fs_window = fs_window_ref.write().unwrap();

            // Use the fs window's existing add_file_tab method which handles everything
            fs_window.add_file_tab(key_details);

            log::info!("Colony: Successfully requested file viewer tab addition to fs window for: {}", address);
        } else {
            log::warn!("Colony: Main FsWindow reference not available for adding file viewer tab");
        }
    }

    /// Format file size for display
    fn format_file_size(size: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size_f = size as f64;
        let mut unit_index = 0;

        while size_f >= 1024.0 && unit_index < UNITS.len() - 1 {
            size_f /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size_f, UNITS[unit_index])
        }
    }

    /// Format date for display
    fn format_date(date_str: &str) -> String {
        // Simple date formatting - extract date and time from ISO 8601 format
        // Expected format: "2024-01-15T10:30:45.123Z" or similar
        if let Some(t_pos) = date_str.find('T') {
            let date_part = &date_str[..t_pos];
            if let Some(colon_pos) = date_str[t_pos..].find(':') {
                let time_part = &date_str[t_pos+1..t_pos+colon_pos+3]; // Get HH:MM
                format!("{} {}", date_part, time_part)
            } else {
                date_part.to_string()
            }
        } else {
            // Fallback to showing the raw string if parsing fails
            date_str.to_string()
        }
    }

    /// Load the user's own contact information
    fn load_user_contact_info(&mut self) {
        if self.is_loading_user_contact {
            return;
        }

        self.is_loading_user_contact = true;
        self.user_contact_info = None;

        let ctx = context();
        wasm_bindgen_futures::spawn_local(async move {
            match ctx.get_user_contact().await {
                Ok(response) => {
                    log::info!("Got user contact info: address={}, type={}, display_name={:?}",
                              response.contact_address, response.contact_type, response.display_name);

                    // Store the response in global state for the UI to pick up
                    let user_info = UserContactInfo {
                        contact_address: response.contact_address,
                        contact_type: response.contact_type,
                        display_name: response.display_name,
                    };

                    if let Ok(mut responses) = USER_CONTACT_RESPONSES.lock() {
                        responses.insert("user_contact".to_string(), user_info);
                    }
                }
                Err(e) => {
                    log::error!("Failed to get user contact info: {:?}", e);
                    // Clear loading state on error by storing an empty response
                    if let Ok(mut responses) = USER_CONTACT_RESPONSES.lock() {
                        responses.insert("user_contact_error".to_string(), UserContactInfo {
                            contact_address: "Error loading contact info".to_string(),
                            contact_type: "error".to_string(),
                            display_name: None,
                        });
                    }
                }
            }
        });
    }
}
