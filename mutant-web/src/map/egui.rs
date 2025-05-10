use bevy::{
    math::Vec3,
    prelude::{Camera, NextState, Query, ResMut, Transform, With},
};
use bevy_egui::EguiContexts;

use crate::app::{
    loading::LoadingWindow, login::LoginWindow, notifications::show_notifications,
    window_system_mut, InfobarWindow,
};

use super::AppState;

pub fn egui_window_system(
    mut contexts: EguiContexts,
    mut infobar: ResMut<InfobarWindow>,
    mut camera: Query<&mut Transform, With<Camera>>,
) {
    infobar.render(contexts.ctx_mut());

    show_notifications(contexts.ctx_mut());

    let Some((x, y)) = window_system_mut().get_focus() else {
        return;
    };

    let Ok(mut camera) = camera.get_single_mut() else {
        return;
    };

    let (x, y) = (x as f32, y as f32);

    camera.translation = Vec3::new(x, y, 0.);
}
pub fn login(
    mut contexts: EguiContexts,
    mut login: ResMut<LoginWindow>,
    state: ResMut<NextState<AppState>>,
) {
    login.render(contexts.ctx_mut(), state);
}

pub fn loading(
    mut contexts: EguiContexts,
    mut loading: ResMut<LoadingWindow>,
    state: ResMut<NextState<AppState>>,
) {
    loading.render(contexts.ctx_mut(), state);
}
