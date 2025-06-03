use serde::{Deserialize, Serialize};
use eframe::egui;
use crate::app::{Window, context::context};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

// Global state for storing user contact info responses
lazy_static::lazy_static! {
    static ref USER_CONTACT_RESPONSES: Arc<Mutex<HashMap<String, UserContactInfo>>> = Arc::new(Mutex::new(HashMap::new()));
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
                    ui.heading("Available Content");
                    
                    // Search bar
                    ui.horizontal(|ui| {
                        ui.label("Search:");
                        ui.text_edit_singleline(&mut self.search_query);
                        if ui.button("🔍").clicked() {
                            self.search_content();
                        }
                        if ui.button("Clear").clicked() {
                            self.search_query.clear();
                            self.refresh_content_list();
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
                                                        egui::RichText::new(format!("from: {}", content.source_contact))
                                                            .size(10.0)
                                                            .color(super::theme::MutantColors::TEXT_MUTED)
                                                    );

                                                    if let Some(size) = content.size {
                                                        ui.label(
                                                            egui::RichText::new(format!("({} bytes)", size))
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

    /// Refresh the content list
    fn refresh_content_list(&mut self) {
        let ctx = context();
        wasm_bindgen_futures::spawn_local(async move {
            match ctx.list_content().await {
                Ok(response) => {
                    log::info!("Content list refreshed: {:?}", response.content);
                    // TODO: Parse content and update UI
                }
                Err(e) => {
                    log::error!("Failed to refresh content list: {:?}", e);
                }
            }
        });
    }

    /// Download content by address
    fn download_content(&self, address: &str) {
        log::info!("Downloading content from address: {}", address);
        // TODO: Implement download functionality
        // This would likely use the existing get functionality with the public address
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
