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

pub fn show_notifications(context: &egui::Context) {
    // Don't modify the style here since we're doing it in the main app
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
