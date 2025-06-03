use serde::{Deserialize, Serialize};
use eframe::egui;
use crate::app::{Window, context::context};

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
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Contact {
    pub pod_address: String,
    pub name: Option<String>,
    pub last_synced: Option<String>,
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
        }
    }
}

impl Window for ColonyWindow {
    fn name(&self) -> String {
        "Colony".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
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
}
