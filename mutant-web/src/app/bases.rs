use bevy_egui::egui::{self, text::LayoutJob, Grid, ProgressBar};
use log::info;
use ogame_core::GAME_DATA;

use crate::game;

use super::{base::BaseWindow, new_window, window_system::request_focus, Window};

use ogame_core::RecipeId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct BasesWindow {}

impl Window for BasesWindow {
    fn name(&self) -> String {
        "Bases".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            egui::RichText::new("Colony Overview")
                .color(egui::Color32::from_rgb(100, 200, 255))
                .size(24.0),
        );
        ui.add_space(8.0);

        let style = ui.style_mut();
        style.visuals.striped = true;

        egui::Grid::new("bases_grid")
            .num_columns(4)
            .spacing([20.0, 12.0])
            .striped(true)
            .show(ui, |ui| {
                // Header row with colored text
                ui.strong(egui::RichText::new("Planet").color(egui::Color32::LIGHT_BLUE));
                ui.strong(egui::RichText::new("Buildings").color(egui::Color32::LIGHT_BLUE));
                ui.strong(egui::RichText::new("Resources").color(egui::Color32::LIGHT_BLUE));
                ui.strong(egui::RichText::new("Actions").color(egui::Color32::LIGHT_BLUE));
                ui.end_row();

                self.content(ui);
            });
    }
}

impl BasesWindow {
    pub fn content(&self, ui: &mut egui::Ui) {
        let bases = &game().bases;

        info!("bases: {:?}", bases);

        for (base_id, base) in bases {
            let planet = GAME_DATA
                .read()
                .planets
                .get(&base.planet_id)
                .unwrap()
                .clone();

            // Planet column
            ui.horizontal(|ui| {
                if ui
                    .link(
                        egui::RichText::new(&planet.name)
                            .color(egui::Color32::from_rgb(150, 200, 255)),
                    )
                    .clicked()
                {
                    request_focus(&planet);
                }
                ui.label(egui::RichText::new(format!("[{}]", planet.x)).color(egui::Color32::GRAY));
            });

            // Buildings column
            let building_count = game()
                .buildings
                .iter()
                .filter(|(_, building)| building.base_id == base.id)
                .count();
            ui.horizontal(|ui| {
                let color = match building_count {
                    0 => egui::Color32::RED,
                    1..=3 => egui::Color32::YELLOW,
                    4..=7 => egui::Color32::from_rgb(150, 200, 100),
                    _ => egui::Color32::GREEN,
                };
                ui.label(
                    egui::RichText::new(format!("{} structures", building_count)).color(color),
                );
            });

            // Resources column
            ui.horizontal(|ui| {
                let resources = &planet.resources;
                if resources.is_empty() {
                    ui.label("No resources");
                } else {
                    Grid::new(format!("resources_grid_{}", base_id))
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            for (recipe_id, richness) in resources {
                                let binding = GAME_DATA.read();
                                let recipe = binding.get_recipe(recipe_id).unwrap();

                                ui.label(&recipe.name);

                                let color = if *richness < 25 {
                                    egui::Color32::RED
                                } else if *richness < 50 {
                                    egui::Color32::ORANGE
                                } else if *richness < 75 {
                                    egui::Color32::YELLOW
                                } else {
                                    egui::Color32::GREEN
                                };

                                let text_color = if *richness < 20 {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::BLACK
                                };

                                let mut text = LayoutJob::default();
                                text.append(
                                    &format!("{:.0}%", richness),
                                    0.0,
                                    egui::TextFormat {
                                        color: text_color,
                                        ..Default::default()
                                    },
                                );

                                let pb = ProgressBar::new(*richness as f32 / 100.0)
                                    .animate(false)
                                    .rounding(0.0)
                                    .text(text)
                                    .fill(color);

                                ui.add(pb);
                                ui.end_row();
                            }
                        });
                }
            });

            // Actions column
            ui.horizontal(|ui| {
                if ui
                    .button(
                        egui::RichText::new("🏢 View Base")
                            .color(egui::Color32::from_rgb(180, 220, 255)),
                    )
                    .clicked()
                {
                    new_window(BaseWindow::new(base_id.clone()));
                }
            });

            ui.end_row();
        }
    }
}
