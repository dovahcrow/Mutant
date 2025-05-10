use bevy_egui::egui;
use ogame_core::GAME_DATA;

use super::{new_window, planet::PlanetWindow, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct PlanetListWindow {}

impl Window for PlanetListWindow {
    fn name(&self) -> String {
        "Planet List".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let planets = GAME_DATA.read().planets.clone();

        for (planet_id, _planet) in planets {
            if ui.button(format!("Planet {}", planet_id)).clicked() {
                let planet_window = PlanetWindow {
                    planet_id: planet_id.clone(),
                };

                new_window(planet_window);
            }
        }
    }
}
