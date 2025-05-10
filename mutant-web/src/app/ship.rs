use bevy_egui::egui::{self, Id};
use ogame_core::{protocol::Protocol, ShipId};

use crate::{game, game_mut};

use super::{
    components::{focusable::focusable, progress::progress},
    inventory::inventory,
    Window,
};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ShipWindow {
    pub ship_id: ShipId,
}
impl Window for ShipWindow {
    fn name(&self) -> String {
        format!("Ship {}", self.ship_id)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.ship_infos("ShipWindow", ui);
    }
}

impl ShipWindow {
    pub fn new(ship_id: ShipId) -> Self {
        Self { ship_id }
    }

    pub fn ship_infos<I: Into<Id>>(&self, salt_id: I, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            let ship = match game().ships.get(&self.ship_id) {
                Some(ship) => ship.clone(),
                None => {
                    ui.label(format!("Ship {} not found", self.ship_id));
                    return;
                }
            };

            self.create_base_button(ui);

            if let Some(flight_id) = &ship.flight_id {
                let flight = match game().flights.get(&flight_id) {
                    Some(flight) => flight.clone(),
                    None => {
                        ui.label("Flight not found");
                        return;
                    }
                };
                let completion = flight.completion();

                let from = match game().get_entity(&flight.from_id) {
                    Ok(entity) => entity.clone(),
                    Err(_) => {
                        ui.label("Source entity not found");
                        return;
                    }
                };

                let to = match game().get_entity(&flight.to_id) {
                    Ok(entity) => entity.clone(),
                    Err(_) => {
                        ui.label("Destination entity not found");
                        return;
                    }
                };

                ui.horizontal(|ui| {
                    focusable(from, ui);
                    ui.label("->");
                    focusable(to, ui);
                });

                ui.add(progress(completion, flight.get_formated_duration()));
            }

            if let Some(position_id) = &ship.position_id {
                match game().get_entity(&position_id) {
                    Ok(entity) => {
                        focusable(entity.clone(), ui);
                    }
                    Err(_) => {
                        ui.label("Position entity not found");
                    }
                }
            }

            let salt = salt_id.into();

            egui::CollapsingHeader::new(format!("Inventory"))
                .id_salt((salt, "Intentory"))
                .show(ui, |ui| {
                    inventory(ship.storage_id.clone(), salt, ui);
                });
        });
    }

    fn create_base_button(&self, ui: &mut egui::Ui) {
        // get storage
        let ship = match game().ships.get(&self.ship_id) {
            Some(ship) => ship.clone(),
            None => return,
        };

        let storage = match game().storages.get(&ship.storage_id) {
            Some(storage) => storage.clone(),
            None => return,
        };

        // if base in inventory
        if storage.has("base") {
            if ui.button("Build Base").clicked() {
                let _ = game_mut().action(Protocol::CreateBase(self.ship_id.clone()));
                ui.close_menu();
            }
        }
    }
}
