use eframe::egui::{self, text::LayoutJob, ProgressBar};

pub fn progress(completion: f32, duration: String) -> ProgressBar {
    let mut text = format!("{:.1}% - {}", completion * 100.0, duration);

    if completion < 0.000001 {
        text = format!("0% - Waiting...");
    } else if completion >= 0.999999 {
        text = format!("100% - Complete");
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

// A more detailed progress bar that shows current/total items
pub fn detailed_progress(completion: f32, current: usize, total: usize, duration: String) -> ProgressBar {
    let mut text = format!("{}/{} - {:.1}% - {}", current, total, completion * 100.0, duration);

    if completion < 0.000001 {
        text = format!("0/{} - Waiting...", total);
    } else if completion >= 0.999999 {
        text = format!("{}/{} - Complete", total, total);
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
