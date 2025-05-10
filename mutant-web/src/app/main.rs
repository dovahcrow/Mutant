use eframe::egui::{self, Color32, RichText};
use humansize::{format_size, BINARY};
use mutant_protocol::KeyDetails;

use super::Window;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MainWindow {
    keys: Vec<KeyDetails>,
    connected: bool,
}

impl Default for MainWindow {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            connected: false,
        }
    }
}

impl Window for MainWindow {
    fn name(&self) -> String {
        "MutAnt Keys".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.draw_keys_list(ui);
    }
}

impl MainWindow {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            connected: false,
        }
    }

    pub fn with_keys(keys: Vec<KeyDetails>, connected: bool) -> Self {
        Self { keys, connected }
    }

    pub fn draw_keys_list(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("MutAnt Storage").size(24.0).color(Color32::from_rgb(100, 200, 255)));

        // Connection status
        ui.horizontal(|ui| {
            let (status_text, status_color) = if self.connected {
                ("Connected", Color32::from_rgb(100, 255, 100))
            } else {
                ("Disconnected", Color32::from_rgb(255, 100, 100))
            };

            ui.label("Status: ");
            ui.label(RichText::new(status_text).color(status_color));
        });

        ui.add_space(12.0);

        if self.keys.is_empty() {
            ui.label(RichText::new("No keys stored.").color(Color32::GRAY));
        } else {
            // Create a table to display the keys
            egui::Grid::new("keys_grid")
                .num_columns(5)
                .spacing([20.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    // Header row
                    ui.strong(RichText::new("Key").color(Color32::LIGHT_BLUE));
                    ui.strong(RichText::new("Size").color(Color32::LIGHT_BLUE));
                    ui.strong(RichText::new("Pads").color(Color32::LIGHT_BLUE));
                    ui.strong(RichText::new("Status").color(Color32::LIGHT_BLUE));
                    ui.strong(RichText::new("Type").color(Color32::LIGHT_BLUE));
                    ui.end_row();

                    // Data rows
                    for key in &self.keys {
                        // Key name
                        ui.label(&key.key);

                        // Size
                        ui.label(format_size(key.total_size, BINARY));

                        // Pads
                        ui.label(format!("{}", key.pad_count));

                        // Status
                        let status_text = format!("{}% ({}/{})",
                            (key.confirmed_pads as f32 / key.pad_count as f32 * 100.0) as u32,
                            key.confirmed_pads,
                            key.pad_count);
                        ui.label(status_text);

                        // Type and address
                        if key.is_public {
                            if let Some(addr) = &key.public_address {
                                let short_addr = if addr.len() > 10 {
                                    format!("{}...{}", &addr[0..5], &addr[addr.len()-5..])
                                } else {
                                    addr.clone()
                                };

                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Public").color(Color32::from_rgb(100, 200, 100)));
                                    if ui.small_button("📋").on_hover_text("Copy address").clicked() {
                                        ui.ctx().copy_text(addr.clone());
                                    }
                                    ui.label(short_addr);
                                });
                            } else {
                                ui.label(RichText::new("Public").color(Color32::from_rgb(100, 200, 100)));
                            }
                        } else {
                            ui.label(RichText::new("Private").color(Color32::from_rgb(200, 100, 100)));
                        }

                        ui.end_row();
                    }
                });
        }

        ui.add_space(12.0);

        // Actions section
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                // This will be implemented later to refresh the keys list
                log::info!("Refresh clicked");
            }

            if ui.button("Sync").clicked() {
                // This will be implemented later to sync with the network
                log::info!("Sync clicked");
            }
        });
    }
}
