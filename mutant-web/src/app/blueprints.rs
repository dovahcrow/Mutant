use bevy_egui::egui;

use crate::game;

use super::{blueprint::BlueprintWindow, new_window, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct BlueprintsWindow {}
impl Window for BlueprintsWindow {
    fn name(&self) -> String {
        "Blueprints".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.blueprints_infos("BlueprintsWindow", ui);
    }
}

impl BlueprintsWindow {
    pub fn new() -> Self {
        Self {}
    }

    pub fn blueprints_infos<I: Into<egui::Id>>(&mut self, _salt_id: I, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            let blueprints = game().blueprints.clone();

            for (id, blueprint) in blueprints {
                ui.horizontal(|ui| {
                    ui.label(blueprint.name.clone());
                    ui.label(blueprint.hull_type.to_string());
                    if ui.button("Open").clicked() {
                        new_window(BlueprintWindow::new(id.clone()));
                    }
                });
            }
        });
    }
}
