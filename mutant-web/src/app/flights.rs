use std::collections::{BTreeMap, HashMap};

use bevy_egui::egui::{self, Grid, Id, RichText};
use log::info;
use ogame_core::{
    flight::MissionType, protocol::Protocol, BaseId, EntityId, NamedEntity, PlanetId, ShipId,
    GAME_DATA,
};

use crate::{game, game_mut};

use super::{
    components::{focusable::focusable, progress::progress},
    new_window,
    ship::ShipWindow,
    Window,
};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct FlightsWindow {
    selected_ships: HashMap<ShipId, bool>,
    selected_destination: Option<EntityId>,
    selected_mission: MissionType,
    selected_base: Option<BaseId>,
}

impl FlightsWindow {
    pub fn new() -> Self {
        let ships = game()
            .ships
            .clone()
            .into_iter()
            .filter(|(_, ship)| ship.flight_id.is_none())
            .map(|(ship_id, _)| (ship_id, false))
            .collect::<HashMap<_, _>>();

        Self {
            selected_ships: ships,
            selected_destination: None,
            selected_mission: MissionType::Transport,
            selected_base: None,
        }
    }

    pub fn with_destination(destination: EntityId) -> Self {
        Self {
            selected_destination: Some(destination),
            ..Self::new()
        }
    }
}

impl Window for FlightsWindow {
    fn name(&self) -> String {
        "Flights".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            ui.heading(RichText::new("Active Flights").size(24.0).strong());
        });
        ui.add_space(12.0);

        self.draw_active_flights(ui);
        ui.add_space(20.0);

        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("New Flight").size(24.0).strong());
        });
        ui.add_space(12.0);

        self.draw_new_flight(ui);
    }
}

impl FlightsWindow {
    fn draw_active_flights(&self, ui: &mut egui::Ui) {
        let flights = game().flights.clone();

        if flights.is_empty() {
            ui.label("No active flights");
            return;
        }

        Grid::new("flights_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .striped(true)
            .show(ui, |ui| {
                for (flight_id, flight) in flights {
                    ui.vertical(|ui| {
                        // Route info
                        ui.horizontal(|ui| {
                            let from_entity = game().get_entity(&flight.from_id).unwrap();
                            let to_entity = game().get_entity(&flight.to_id).unwrap();

                            focusable(from_entity.clone(), ui);
                            ui.label("→");
                            focusable(to_entity.clone(), ui);
                        });

                        // Mission and ships
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("🎯 {}", flight.mission)).strong());
                            ui.label("|");
                            ui.label(format!("🚀 {} ships", flight.ships.len()));
                        });
                    });

                    // Progress
                    ui.vertical(|ui| {
                        let completion = flight.completion();
                        ui.add(progress(completion, flight.get_formated_duration()));
                    });

                    ui.end_row();
                }
            });
    }

    fn draw_new_flight(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            // Mission type selector with improved styling
            ui.horizontal(|ui| {
                ui.label(RichText::new("Mission:").size(16.0).strong());
                ui.add_space(8.0);
                egui::ComboBox::from_label("")
                    .selected_text(
                        RichText::new(format!("🎯 {}", self.selected_mission)).size(16.0),
                    )
                    .show_ui(ui, |ui| {
                        ui.style_mut().wrap = Some(false);
                        ui.selectable_value(
                            &mut self.selected_mission,
                            MissionType::Transport,
                            RichText::new("🚛 Transport").size(16.0),
                        );
                        ui.selectable_value(
                            &mut self.selected_mission,
                            MissionType::Attack(
                                self.selected_base
                                    .clone()
                                    .unwrap_or_else(|| BaseId("0".to_string())),
                            ),
                            RichText::new("⚔️ Attack").size(16.0),
                        );
                        ui.selectable_value(
                            &mut self.selected_mission,
                            MissionType::Colonize,
                            RichText::new("🌍 Colonize").size(16.0),
                        );
                        ui.selectable_value(
                            &mut self.selected_mission,
                            MissionType::Espionage,
                            RichText::new("🔍 Espionage").size(16.0),
                        );
                        ui.selectable_value(
                            &mut self.selected_mission,
                            MissionType::Station,
                            RichText::new("🛰️ Station").size(16.0),
                        );
                    });
            });

            ui.add_space(16.0);

            // Destination selector
            ui.horizontal(|ui| {
                ui.label(RichText::new("Destination:").size(16.0).strong());
                ui.add_space(8.0);
                if let Some(dest) = self.selected_destination.clone() {
                    let entity = game().get_entity(&dest).unwrap();
                    if ui
                        .button(RichText::new(format!("🌍 {}", entity.name())).size(16.0))
                        .clicked()
                    {
                        self.selected_destination = None;
                        self.selected_base = None;
                    }
                } else {
                    let popup_id = Id::new("destination_picker");
                    let button_response =
                        ui.button(RichText::new("📍 Select destination").size(16.0));
                    if button_response.clicked() {
                        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                    }

                    egui::popup::popup_below_widget(
                        ui,
                        popup_id,
                        &button_response,
                        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                        |ui| {
                            if let Some(dest) = chose_destination(ui) {
                                self.selected_destination = Some(dest);
                                self.selected_base = None;
                            }
                        },
                    );
                }
            });

            // Base selector
            if matches!(self.selected_mission, MissionType::Attack(_))
                && self.selected_destination.is_some()
            {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Target Base:").size(16.0).strong());
                    ui.add_space(8.0);
                    if let Some(base_id) = &self.selected_base {
                        if let Some(base) = game()
                            .other_players
                            .values()
                            .flat_map(|p| p.bases.values())
                            .find(|b| b.id == *base_id)
                        {
                            if ui
                                .button(
                                    RichText::new(format!(
                                        "🏠 {}'s base",
                                        game().other_players[&base.user_id].username
                                    ))
                                    .size(16.0),
                                )
                                .clicked()
                            {
                                self.selected_base = None;
                            }
                        }
                    } else {
                        let popup_id = Id::new("base_picker");
                        let button_response =
                            ui.button(RichText::new("🎯 Select target base").size(16.0));
                        if button_response.clicked() {
                            ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                        }

                        egui::popup::popup_below_widget(
                            ui,
                            popup_id,
                            &button_response,
                            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                if let Some(dest) = &self.selected_destination {
                                    for player in game().other_players.values() {
                                        for (base_id, base) in &player.bases {
                                            if let Ok(planet_id) = dest.planet_id() {
                                                if base.planet_id == planet_id {
                                                    if ui
                                                        .button(
                                                            RichText::new(format!(
                                                                "🏠 {}'s base",
                                                                player.username
                                                            ))
                                                            .size(16.0),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.selected_base = Some(base_id.clone());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                        );
                    }
                });
            }

            ui.add_space(16.0);

            // Ship selection
            ui.label(RichText::new("Select ships:").size(16.0).strong());
            ui.add_space(8.0);
            let ships_by_position = self.get_ships_by_position();

            for (position_name, ships) in &ships_by_position {
                egui::CollapsingHeader::new(
                    RichText::new(format!("📍 {}", position_name)).size(16.0),
                )
                .default_open(true)
                .show(ui, |ui| {
                    Grid::new(format!("ships_at_{}", position_name))
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            for (ship, _) in ships {
                                ui.checkbox(self.selected_ships.get_mut(&ship).unwrap(), "");

                                let label = ui.selectable_label(
                                    false,
                                    RichText::new(ship.to_string())
                                        .text_style(egui::TextStyle::Monospace)
                                        .size(14.0),
                                );

                                if label.clicked() {
                                    new_window(ShipWindow {
                                        ship_id: ship.clone(),
                                    });
                                }

                                ui.end_row();
                            }
                        });
                });
            }

            ui.add_space(20.0);

            // Launch button
            let send_enabled = !self.selected_ships.is_empty()
                && self.selected_ships.iter().any(|(_, selected)| *selected)
                && self.selected_destination.is_some()
                && (!matches!(self.selected_mission, MissionType::Attack(_))
                    || self.selected_base.is_some());

            ui.add_enabled_ui(send_enabled, |ui| {
                ui.vertical_centered(|ui| {
                    if ui
                        .add_sized(
                            [120.0, 32.0],
                            egui::Button::new(RichText::new("🚀 Launch Fleet").size(16.0).strong()),
                        )
                        .clicked()
                    {
                        self.send_fleet();
                    }
                });
            });
        });
    }

    fn get_ships_by_position(&self) -> BTreeMap<String, Vec<(ShipId, bool)>> {
        game()
            .ships
            .clone()
            .into_iter()
            .filter(|(_, ship)| ship.flight_id.is_none())
            .map(|(ship_id, ship)| {
                let position_name = ship
                    .position_id
                    .as_ref()
                    .map(|pos| game().get_entity(pos).unwrap().name())
                    .unwrap_or_else(|| "Unknown".to_string());
                (position_name, (ship_id, false))
            })
            .fold(BTreeMap::new(), |mut acc, (pos, ship)| {
                acc.entry(pos).or_default().push(ship);
                acc
            })
    }

    fn send_fleet(&mut self) {
        let selected_ships = self
            .selected_ships
            .iter()
            .filter(|(_, selected)| **selected)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();

        let from_id = game()
            .ships
            .get(&selected_ships[0])
            .and_then(|ship| ship.position_id.clone())
            .expect("Ship must have a position to launch");

        match self.selected_mission {
            MissionType::Attack(_) => {
                game_mut()
                    .action(Protocol::SendShips {
                        from_id: from_id.into(),
                        to_id: self.selected_destination.clone().unwrap(),
                        ships_ids: selected_ships,
                        mission: MissionType::Attack(self.selected_base.clone().unwrap()),
                        speed_ratio: 1,
                    })
                    .unwrap();
            }
            _ => {
                game_mut()
                    .action(Protocol::SendShips {
                        from_id: from_id.into(),
                        to_id: self.selected_destination.clone().unwrap(),
                        ships_ids: selected_ships,
                        mission: self.selected_mission.clone(),
                        speed_ratio: 1,
                    })
                    .unwrap();
            }
        }

        // Reset selection
        self.selected_destination = None;
        self.selected_base = None;
        self.selected_ships
            .iter_mut()
            .for_each(|(_, selected)| *selected = false);
    }
}

fn chose_destination(ui: &mut egui::Ui) -> Option<EntityId> {
    let game_data = GAME_DATA.read();
    let mut selected = None;

    let mut systems = game_data.systems.clone().into_iter().collect::<Vec<_>>();
    systems.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name));

    ui.collapsing("Destination", |ui| {
        for (system_id, system) in &systems {
            let mut planets = game_data
                .planets
                .iter()
                .filter(|(_, planet)| &planet.system_id == system_id)
                .collect::<Vec<_>>();

            planets.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name));

            ui.collapsing(system.name.clone(), |ui| {
                for (planet_id, planet) in planets {
                    if ui.button(format!("Planet {}", planet.name)).clicked() {
                        selected = Some(planet_id.clone().into());
                    }
                }
            });
        }
    });

    selected
}
