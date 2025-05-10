use bevy_egui::egui;

use super::{components::chart::Chart, Window};

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ChartWindow {
    chart: Chart,
    pub id: u64,
}

impl ChartWindow {
    pub fn new() -> Self {
        Self {
            id: 0,
            chart: Chart::new(),
        }
    }
}

impl Window for ChartWindow {
    fn name(&self) -> String {
        format!("Chart {}", self.id)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::neither()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                self.chart.box_plot(ui);
            });
    }
}
