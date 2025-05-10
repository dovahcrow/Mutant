use bevy_egui::egui::{self, Color32};
use ogame_core::{building::Building, protocol::Protocol, BaseId, BuildingId, GAME_DATA};

use crate::{game, game_mut};

use super::{components::progress::progress, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct BuildingsWindow {
    pub base_id: BaseId,
    pub open_picker: Option<BuildingId>,

    toggle_open: Option<bool>,
}

impl Window for BuildingsWindow {
    fn name(&self) -> String {
        format!("Buildings {}", self.base_id)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.draw_buildings(ui, self.toggle_open);
    }
}

impl BuildingsWindow {
    pub fn new(base_id: BaseId) -> Self {
        Self {
            base_id,
            open_picker: None,
            toggle_open: Some(true),
        }
    }

    pub fn draw_buildings(&mut self, ui: &mut egui::Ui, toggle_open: Option<bool>) {
        ui.add_space(8.0);
        self.draw_building_buttons(ui);
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        let base = game().get_base(&self.base_id).unwrap().clone();
        let buildings = game()
            .buildings
            .clone()
            .into_iter()
            .filter(|(_, building)| building.base_id == base.id)
            .collect::<Vec<_>>();

        let buildings_by_name = buildings.iter().fold(
            std::collections::BTreeMap::new(),
            |mut acc, (_building_id, building)| {
                acc.entry(building.name.clone())
                    .or_insert_with(Vec::new)
                    .push(building.clone());
                acc
            },
        );

        for (name, buildings) in buildings_by_name {
            let building_count = buildings.len();
            egui::CollapsingHeader::new(format!("{} ({})", name, building_count))
                .open(toggle_open)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    if name == "shipyard" {
                        self.factory_shipyards(buildings, ui, toggle_open);
                    } else {
                        self.factory_buildings(buildings, ui, toggle_open);
                    }
                    ui.add_space(4.0);
                });
            ui.add_space(8.0);
        }
    }

    fn factory_shipyards(
        &mut self,
        buildings: Vec<Building>,
        ui: &mut egui::Ui,
        _toggle_open: Option<bool>,
    ) {
        for shipyard in buildings {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let recipe_name = match &shipyard.recipe_id {
                        Some(recipe_id) => recipe_id.clone(),
                        None => "⚡ Click to assign blueprint".to_string(),
                    };

                    let status_color = if shipyard.has_output {
                        egui::Color32::from_rgb(220, 50, 50)
                    } else if shipyard.recipe_id.is_some() {
                        egui::Color32::from_rgb(50, 220, 50)
                    } else {
                        egui::Color32::from_rgb(150, 150, 150)
                    };

                    ui.scope(|ui| {
                        ui.set_width(200.0);
                        if ui
                            .add(
                                egui::Button::new(recipe_name)
                                    .fill(status_color.linear_multiply(0.2)),
                            )
                            .clicked()
                        {
                            self.open_picker = Some(shipyard.id.clone());
                        }
                    });

                    if shipyard.recipe_id.is_some() {
                        let completion = shipyard.completion();
                        let mut progress_bar =
                            progress(completion, shipyard.get_formated_duration()).animate(true);

                        if shipyard.has_output {
                            progress_bar = progress_bar.fill(status_color);
                        }

                        ui.add(progress_bar);
                    }
                });

                // Show queued ships
                for (_, shipyard_queue) in game().shipyard_queues.iter() {
                    if shipyard_queue.shipyard_id == shipyard.id {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("");
                            ui.label(format!("Queued: {}", shipyard_queue.blueprint_id));
                            let completion = shipyard_queue.completion();
                            ui.add(
                                progress(completion, shipyard_queue.get_formated_duration())
                                    .animate(true),
                            );
                        });
                    }
                }

                if let Some(open_picker) = &self.open_picker {
                    if open_picker == &shipyard.id {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        if draw_blueprint_picker(shipyard.id.clone(), ui) {
                            self.open_picker = None;
                        }
                    }
                }
            });
            ui.add_space(8.0);
        }
    }

    fn factory_buildings(
        &mut self,
        buildings: Vec<Building>,
        ui: &mut egui::Ui,
        toggle_open: Option<bool>,
    ) {
        let buildings_by_recipe =
            buildings
                .iter()
                .fold(std::collections::BTreeMap::new(), |mut acc, building| {
                    acc.entry(building.recipe_id.clone())
                        .or_insert_with(Vec::new)
                        .push(building.clone());
                    acc
                });

        for (recipe, buildings) in buildings_by_recipe {
            let recipe_name = recipe.unwrap_or_else(|| "Idle".to_string());
            let count = buildings.len();

            egui::CollapsingHeader::new(format!("{} ({})", recipe_name, count))
                .open(toggle_open)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    for building in buildings {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let status_color = if building.has_output {
                                    egui::Color32::from_rgb(220, 50, 50)
                                } else if building.recipe_id.is_some() {
                                    egui::Color32::from_rgb(50, 220, 50)
                                } else {
                                    egui::Color32::from_rgb(150, 150, 150)
                                };

                                ui.scope(|ui| {
                                    ui.set_width(200.0);
                                    if ui
                                        .add(
                                            egui::Button::new(format!("🏭 {}", building.name))
                                                .fill(status_color.linear_multiply(0.2)),
                                        )
                                        .clicked()
                                    {
                                        self.open_picker = Some(building.id.clone());
                                    }
                                });

                                if building.recipe_id.is_some() {
                                    let completion = building.completion();
                                    let mut progress_bar =
                                        progress(completion, building.get_formated_duration())
                                            .animate(true);

                                    if building.has_output {
                                        progress_bar = progress_bar.fill(status_color);
                                    }

                                    ui.add(progress_bar);
                                }
                            });

                            if let Some(open_picker) = &self.open_picker {
                                if open_picker == &building.id {
                                    ui.add_space(8.0);
                                    ui.separator();
                                    ui.add_space(8.0);
                                    if draw_recipe_picker(building.id.clone(), ui) {
                                        self.open_picker = None;
                                    }
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(8.0);
        }
    }

    fn draw_building_buttons(&self, ui: &mut egui::Ui) {
        let base = game().bases.get(&self.base_id).unwrap().clone();
        let storage = game().get_storage(&base.storage_id).unwrap().clone();
        let buildable_buildings = GAME_DATA.read().get_buildable_buildings();

        if buildable_buildings.is_empty() {
            return;
        }

        ui.heading("Create New Building");
        ui.add_space(8.0);

        egui::Grid::new("buildable_buildings")
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                let mut count = 0;
                for building_recipe in &buildable_buildings {
                    if storage.has(building_recipe) {
                        if count > 0 && count % 3 == 0 {
                            ui.end_row();
                        }
                        if ui
                            .add(
                                egui::Button::new(format!(
                                    "🏗 {} ({})",
                                    building_recipe,
                                    storage.items.get(building_recipe).unwrap()
                                ))
                                .min_size(egui::vec2(150.0, 30.0))
                                .fill(Color32::from_rgb(100, 150, 255).linear_multiply(0.2)),
                            )
                            .clicked()
                        {
                            game_mut()
                                .action(Protocol::CreateBuilding(
                                    base.id.clone(),
                                    building_recipe.clone(),
                                ))
                                .unwrap();
                        }
                        count += 1;
                    }
                }
            });
    }
}

fn draw_recipe_picker(building_id: BuildingId, ui: &mut egui::Ui) -> bool {
    ui.heading("Available Recipes");
    ui.add_space(8.0);

    let game_data = GAME_DATA.read();
    let game = game();
    let building = game.get_building(&building_id).unwrap();

    let recipes = game_data
        .recipes
        .values()
        .filter(|recipe| {
            if let Some(technology) = &recipe.technology {
                game.technologies.contains(technology)
            } else {
                true
            }
        })
        // filter the recipes that have richness == 0 on the planet
        .filter(|recipe| {
            if recipe.inputs.is_empty() {
                let planet = game.get_base(&building.base_id).unwrap().planet_id.clone();
                let game_data = GAME_DATA.read();
                let richness = game_data
                    .planets
                    .get(&planet)
                    .unwrap()
                    .resources
                    .get(&recipe.name)
                    .unwrap_or(&0);

                *richness > 0
            } else {
                true
            }
        })
        .filter(|recipe| recipe.made_in == building.name)
        .collect::<Vec<_>>();

    for recipe in recipes {
        let inputs = recipe
            .inputs
            .iter()
            .map(|(recipe_id, amount)| format!("{} {}", amount, recipe_id))
            .collect::<Vec<_>>()
            .join(" + ");

        if ui
            .add(
                egui::Button::new(format!("{} = {}", recipe.name, inputs))
                    .fill(Color32::from_rgb(130, 180, 255).linear_multiply(0.15)),
            )
            .clicked()
        {
            // TODO: handle error
            let _ = game_mut().action(Protocol::ChangeRecipe {
                building_id: building.id.clone(),
                recipe_id: recipe.name.clone(),
            });

            return true;
        }
    }

    false
}

fn draw_blueprint_picker(shipyard_id: BuildingId, ui: &mut egui::Ui) -> bool {
    ui.heading("Available Blueprints");
    ui.add_space(8.0);

    let shipyard = game().get_building(&shipyard_id).unwrap().clone();
    let blueprints = game().blueprints.clone();

    for (_id, blueprint) in blueprints {
        if ui
            .add(
                egui::Button::new(format!("{}", blueprint.name))
                    .fill(Color32::from_rgb(100, 170, 255).linear_multiply(0.2)),
            )
            .clicked()
        {
            let _ = game_mut().action(Protocol::QueueShip {
                shipyard_id: shipyard.id.clone(),
                blueprint_id: blueprint.id.clone(),
            });

            return true;
        }
    }

    false
}
