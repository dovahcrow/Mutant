use std::sync::{Arc, RwLock};

use bevy::prelude::{NextState, ResMut, Resource};
use bevy_egui::egui;
use egui_dock::egui::{vec2, Context, Margin, Vec2b};
use ogame_core::game::Faction;

use crate::{init::send_auth, map::AppState};

use super::Window;

use serde::{Deserialize, Serialize};
#[derive(Resource, Default, Clone, Serialize, Deserialize)]
pub struct LoginWindow {
    pub email: String,
    pub username: String,
    pub password: String,
    pub is_register: bool,
    pub is_logged: Arc<RwLock<bool>>,
    pub is_init: Arc<RwLock<bool>>,
    pub faction: Faction,
    pub error: Arc<RwLock<String>>,
}

impl Window for LoginWindow {
    fn name(&self) -> String {
        "Login".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().spacing.item_spacing = egui::vec2(10.0, 10.0);
        ui.add_space(20.0);

        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label("Email");
                ui.text_edit_singleline(&mut self.email);

                if self.is_register {
                    ui.label("Username");
                    ui.text_edit_singleline(&mut self.username);
                }

                ui.label("Password");
                ui.text_edit_singleline(&mut self.password);

                if self.is_register {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.faction, Faction::Faction1, "Faction1");
                        ui.selectable_value(&mut self.faction, Faction::Faction2, "Faction2");
                        ui.selectable_value(&mut self.faction, Faction::Faction3, "Faction3");
                    });

                    if ui.button("Register").clicked() {
                        self.auth();
                    }

                    if ui.button("Already have an account ?").clicked() {
                        self.is_register = false;
                        self.username.clear();
                    }
                } else {
                    if ui.button("Login").clicked() {
                        self.auth();
                    }

                    if ui.button("Don't have an account ?").clicked() {
                        self.is_register = true;
                    }
                }

                ui.colored_label(egui::Color32::RED, &*self.error.read().unwrap());
            });
        });
    }
}
impl LoginWindow {
    fn auth(&mut self) {
        let email = self.email.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let faction = self.faction.clone();
        let error = self.error.clone();

        let method = if self.is_register {
            "register".to_string()
        } else {
            "login".to_string()
        };

        let is_logged = self.is_logged.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let res = send_auth(
                method,
                email.clone(),
                username.clone(),
                password.clone(),
                faction.clone(),
            )
            .await;

            if let Err(e) = res {
                log::error!("error: {:?}", e);
                *error.write().unwrap() = format!("{:?}", e).to_string();
                return;
            }

            *is_logged.write().unwrap() = true;
        });
    }

    pub fn render(&mut self, ctx: &mut Context, mut next_state: ResMut<NextState<AppState>>) {
        let mut frame = egui::Frame::default().fill(egui::Color32::from_black_alpha(200));
        frame.inner_margin = Margin::from(vec2(10.0, 10.0));

        egui::Window::new("Login")
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

                if *self.is_logged.read().unwrap() && !*self.is_init.read().unwrap() {
                    *self.is_logged.write().unwrap() = false;
                    next_state.set(AppState::LoadingData);
                }
            });
    }
}
