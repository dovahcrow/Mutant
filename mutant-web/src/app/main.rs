use eframe::egui;


use super::{
    new_window,
    Window,
};

use serde::{Deserialize, Serialize};
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct MainWindow {
    // pub base_id: BaseId,
    // pub toggle_open: Option<bool>,
    // pub buildings_window: BuildingsWindow,
}
impl Window for MainWindow {
    fn name(&self) -> String {
        "Main".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        // self.base_infos(ui);
    }
}

impl MainWindow {
    pub fn new() -> Self {
        Self {
            // buildings_window: BuildingsWindow::new(base_id.clone()),
            // base_id,
            // toggle_open: Some(true),
        }
    }

    pub fn base_infos(&mut self, ui: &mut egui::Ui) {
        if ui.button("Buildings").clicked() {
            // new_window(BuildingsWindow::new(self.base_id.clone()));
        }
        // egui::Grid::new(format!("base{}", self.base_id))
        //     .num_columns(2)
        //     .spacing([40.0, 4.0])
        //     .striped(true)
        //     .show(ui, |ui| {
        //         let base = game().bases.get(&self.base_id).unwrap().clone();

        //         let planet = GAME_DATA
        //             .read()
        //             .planets
        //             .get(&base.planet_id)
        //             .unwrap()
        //             .clone();

        //         ui.label("Planet");
        //         focusable(planet.clone(), ui);
        //         ui.end_row();

        //         ui.label("Inventory");
        //         inventory(
        //             base.storage_id.clone(),
        //             format!("base_storage{}", base.storage_id),
        //             ui,
        //         );
        //         ui.end_row();
        //     });
    }

    // pub fn base_infos_overview<I: Into<egui::Id>>(&mut self, salt_id: I, ui: &mut egui::Ui) {
    //     let base = game().bases.get(&self.base_id).unwrap().clone();

    //     let planet = GAME_DATA
    //         .read()
    //         .planets
    //         .get(&base.planet_id)
    //         .unwrap()
    //         .clone();

    //     focusable(planet.clone(), ui);

    //     let salt = salt_id.into();

    //     egui::CollapsingHeader::new("Inventory")
    //         .id_salt(format!("storage{}", base.storage_id))
    //         .show(ui, |ui| {
    //             inventory(base.storage_id, salt, ui);
    //         });

    //     egui::CollapsingHeader::new("Buildings")
    //         .id_salt(format!("buildings{}", base.id))
    //         .open(self.toggle_open)
    //         .show(ui, |ui| {
    //             self.buildings_window.draw_buildings(ui, self.toggle_open);
    //         });

    //     egui::CollapsingHeader::new("Ships")
    //         .id_salt(format!("ships{}", base.id))
    //         .open(self.toggle_open)
    //         .show(ui, |ui| {
    //             ShipsWindow {}.ships_at(base.id.clone().into(), salt, ui);
    //         });
    // }
}
