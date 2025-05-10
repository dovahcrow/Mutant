use std::collections::BTreeMap;

use bevy_egui::egui::{self, Color32, Id, RichText, Stroke};
use ogame_core::{ship::Ship, EntityId};

use crate::game;

use super::{ship::ShipWindow, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ShipsWindow {}

impl ShipsWindow {
    fn draw_ship_group(
        &self,
        mut ships: Vec<Ship>,
        title: &str,
        color: Color32,
        ui: &mut egui::Ui,
    ) {
        if ships.is_empty() {
            return;
        }

        // Sort ships by ID for deterministic ordering
        ships.sort_by_key(|ship| ship.id.0.clone());

        egui::CollapsingHeader::new(RichText::new(title).strong().color(color))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for ship in ships {
                    let id = ship.id.0.clone();
                    ui.push_id(id, |ui| {
                        ShipWindow::new(ship.id.clone()).ship_infos("ShipsWindow", ui);
                    });
                    ui.add_space(2.0);
                }
            });
        ui.add_space(4.0);
    }

    fn group_ships_by_position(
        ships: &BTreeMap<ogame_core::ShipId, Ship>,
    ) -> (Vec<Ship>, BTreeMap<EntityId, Vec<Ship>>) {
        let mut ships_by_position =
            ships
                .iter()
                .fold(BTreeMap::new(), |mut acc, (_ship_id, ship)| {
                    acc.entry(ship.position_id.clone())
                        .or_insert_with(Vec::new)
                        .push(ship.clone());
                    acc
                });

        let inflight = ships_by_position.remove(&None).unwrap_or_default();

        let ships_by_position = ships_by_position
            .into_iter()
            .filter(|(position_opt, _)| position_opt.is_some())
            .map(|(position_opt, ships)| (position_opt.unwrap(), ships))
            .collect();

        (inflight, ships_by_position)
    }
}

impl Window for ShipsWindow {
    fn name(&self) -> String {
        "Ships".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let ships = game().ships.clone();
        let (inflight, ships_by_position) = Self::group_ships_by_position(&ships);

        ui.add_space(8.0);
        egui::Frame::none()
            .stroke(Stroke::new(1.0, Color32::from_gray(100)))
            .show(ui, |ui| {
                // Draw in-flight ships
                self.draw_ship_group(inflight, "🚀 In Flight", Color32::LIGHT_BLUE, ui);

                // Draw ships at each position
                for (position_id, ships) in ships_by_position {
                    self.draw_ship_group(
                        ships,
                        &format!("📍 Position {}", position_id),
                        Color32::LIGHT_GREEN,
                        ui,
                    );
                }
            });
    }
}

impl ShipsWindow {
    pub fn ships_at<I: Into<Id>>(&mut self, entity_id: EntityId, salt_id: I, ui: &mut egui::Ui) {
        let ships = game().ships.clone();
        let salt = salt_id.into();
        let (_inflight, ships_by_position) = Self::group_ships_by_position(&ships);

        // Only show ships at this position
        if let Some(ships_at_position) = ships_by_position.get(&entity_id) {
            let mut ships = ships_at_position.clone();
            ships.sort_by_key(|ship| ship.id.0.clone());

            egui::CollapsingHeader::new(
                RichText::new(format!("📍 Position {}", entity_id))
                    .strong()
                    .color(Color32::LIGHT_GREEN),
            )
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(2.0);
                for ship in ships {
                    let id = ship.id.0.clone();
                    ShipWindow::new(ship.id.clone()).ship_infos(format!("{:?}{}", salt, id), ui);
                }
            });
            ui.add_space(4.0);
        }
    }
}
