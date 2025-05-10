use bevy_egui::egui;
use ogame_core::{BlueprintId, GAME_DATA};

use crate::game;

use super::Window;

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct BlueprintWindow {
    pub blueprint_id: BlueprintId,
}
impl Window for BlueprintWindow {
    fn name(&self) -> String {
        format!("Blueprint {}", self.blueprint_id)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.blueprint_infos(self.name(), ui);
    }
}

impl BlueprintWindow {
    pub fn new(blueprint_id: BlueprintId) -> Self {
        Self { blueprint_id }
    }

    pub fn blueprint_infos<I: Into<egui::Id>>(&mut self, _salt_id: I, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            let blueprint = game().get_blueprint(&self.blueprint_id).unwrap().clone();

            let hull = GAME_DATA
                .read()
                .get_hull(&blueprint.hull_type)
                .unwrap()
                .clone();

            ui.label(blueprint.name.clone());

            for (slot, part) in hull.slots.iter().zip(blueprint.parts.iter()) {
                ui.horizontal(|ui| {
                    ui.label(format!("{:?}", slot.slot_type));
                    ui.label(format!("{:?}", part));
                });
            }
        });
    }
}
