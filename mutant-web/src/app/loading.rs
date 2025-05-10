use std::sync::{Arc, RwLock};

use bevy::prelude::{NextState, ResMut, Resource};
use bevy_egui::egui;
use egui_dock::egui::{vec2, Context, Margin, Vec2b};
use log::info;

use crate::{
    init::game_init,
    map::{AppState, LoadingInfo},
};

use super::{components::progress::progress, Window};

#[derive(Resource, Clone)]
pub struct LoadingWindow {
    pub infos: Arc<RwLock<LoadingInfo>>,
    pub first_run: bool,
    pub is_ready: Arc<RwLock<bool>>,
}

impl Default for LoadingWindow {
    fn default() -> Self {
        Self {
            infos: Arc::new(RwLock::new(LoadingInfo::default())),
            first_run: true,
            is_ready: Arc::new(RwLock::new(false)),
        }
    }
}

impl Window for LoadingWindow {
    fn name(&self) -> String {
        "Loading".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        // ui.style_mut().spacing.item_spacing = egui::vec2(10.0, 10.0);

        // ui.style_mut().spacing.window_margin = Margin::from(vec2(0.0, 50.0));
        ui.add_space(50.0);
        ui.vertical_centered(|ui| {
            let loading_infos = self.infos.read().unwrap();
            if loading_infos.progress < 1.0 {
                let progress_bar = progress(loading_infos.progress, "".to_string());

                ui.add(progress_bar);
            }
            ui.label(loading_infos.message.clone());
        });
    }
}
impl LoadingWindow {
    pub fn render(&mut self, ctx: &mut Context, mut next_state: ResMut<NextState<AppState>>) {
        let mut frame = egui::Frame::default().fill(egui::Color32::from_black_alpha(0));
        frame.inner_margin = Margin::from(vec2(10.0, 10.0));

        egui::Window::new("Loading")
            .resizable([false, false])
            .frame(frame)
            .constrain(true)
            .collapsible(false)
            .title_bar(false)
            .scroll(Vec2b::TRUE)
            .fixed_size(egui::Vec2::new(600.0, 200.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                self.draw(ui);

                // ui.style_mut().visuals.window_fill = egui::Color32::from_black_alpha(0);
                if *self.is_ready.read().unwrap() {
                    *self.is_ready.write().unwrap() = false;
                    info!("SWITCH LOADING SCENE");
                    next_state.set(AppState::LoadingScene);
                }

                if self.first_run {
                    self.first_run = false;

                    let infos = self.infos.clone();
                    let is_ready = self.is_ready.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ready_tx = game_init(infos).await;
                        // next_state.set(AppState::LoadingData);
                        *is_ready.write().unwrap() = true;
                    });
                }
            });
    }
}
