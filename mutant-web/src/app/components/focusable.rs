use egui_dock::egui::Ui;
use ogame_core::Entity;

use crate::app::{flights::FlightsWindow, new_window, window_system::request_focus};

pub fn focusable<T: Into<Entity>>(entity: T, ui: &mut Ui) {
    let entity: Entity = entity.into();
    let response = ui.link(&entity.name());

    if response.clicked() {
        request_focus(&entity);
    }

    response.context_menu(|menu| {
        if menu.button("Fly to").clicked() {
            new_window(FlightsWindow::with_destination(entity.id()));
            menu.close_menu();
        }
    });
}
