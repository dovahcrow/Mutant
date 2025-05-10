use bevy_egui::egui;
use egui_dock::egui::{text::LayoutJob, Grid, ProgressBar, TextFormat};
use ogame_core::{PlanetId, PositionedEntity, GAME_DATA};

use super::{components::focusable::focusable, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct PlanetWindow {
    pub planet_id: PlanetId,
}

impl Window for PlanetWindow {
    fn name(&self) -> String {
        let planet = GAME_DATA
            .read()
            .planets
            .get(&self.planet_id)
            .unwrap()
            .clone();

        format!("Planet {}", planet.name)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let planet = GAME_DATA
            .read()
            .planets
            .get(&self.planet_id)
            .unwrap()
            .clone();

        let (x, y) = planet.get_real_position();

        ui.vertical(|ui| {
            Grid::new(format!("planet_{}", self.planet_id))
                .num_columns(2)
                .spacing([40.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Name");
                    focusable(planet.clone(), ui);
                    ui.end_row();

                    ui.label("Coordinates");
                    ui.label(format!("({}, {})", x, y));
                    ui.end_row();

                    ui.label("Resources");

                    Grid::new("my_grid")
                        .num_columns(2)
                        .spacing([40.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for (recipe_id, richness) in &planet.resources {
                                ui.label(format!("{}", recipe_id));

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
                                    TextFormat {
                                        color: text_color,
                                        ..Default::default()
                                    },
                                );

                                let pb = ProgressBar::new(*richness as f32 / 100.0)
                                    .animate(false)
                                    .rounding(0.0)
                                    // .show_percentage()
                                    .text(text)
                                    .fill(color);

                                ui.add(pb);

                                ui.end_row();
                            }
                        });

                    ui.end_row();
                });
        });
    }
}
