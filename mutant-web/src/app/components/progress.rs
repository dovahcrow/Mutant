use eframe::egui::{self, text::LayoutJob, ProgressBar};
use egui_dock::egui::{FontId, TextFormat};

pub fn progress(completion: f32, duration: String) -> ProgressBar {
    let mut text = format!("{:.2}% - {}", completion * 100.0, duration,);

    if completion < 0.000001 {
        text = format!("Missing input");
    }

    let mut job = LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            valign: egui::Align::Center,
            ..Default::default()
        },
    );

    job.halign = egui::Align::Center;

    ProgressBar::new(completion as f32).animate(true).text(job)
}
