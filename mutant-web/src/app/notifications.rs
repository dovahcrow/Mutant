use std::sync::RwLock;

use eframe::egui::{self, Align2};
use egui_toast::Toasts;
use lazy_static::lazy_static;

lazy_static! {
    static ref NOTIFICATIONS: RwLock<Toasts> = RwLock::new(
        Toasts::new()
        .anchor(Align2::RIGHT_TOP, (-10.0, -10.0)) // 10 units from the bottom right corner
    .direction(egui::Direction::BottomUp));
}

pub fn show_notifications(context: &mut egui::Context) {
    context.style_mut(|style| {
        style.visuals.window_fill = egui::Color32::from_rgb(0, 0, 0);
    });
    NOTIFICATIONS.write().unwrap().show(context);
}

pub fn notification(toast: egui_toast::Toast) {
    NOTIFICATIONS.write().unwrap().add(
        toast.options(
            egui_toast::ToastOptions::default()
                .duration_in_seconds(5.0)
                .show_icon(true)
                .show_progress(true),
        ),
    );
}

#[allow(unused)]
pub fn error(message: String) {
    notification(
        egui_toast::Toast::new()
            .kind(egui_toast::ToastKind::Error)
            .text(message),
    );
}

#[allow(unused)]
pub fn warning(message: String) {
    notification(
        egui_toast::Toast::new()
            .kind(egui_toast::ToastKind::Warning)
            .text(message),
    );
}

#[allow(unused)]
pub fn info(message: String) {
    notification(
        egui_toast::Toast::new()
            .kind(egui_toast::ToastKind::Info)
            .text(message),
    );
}
