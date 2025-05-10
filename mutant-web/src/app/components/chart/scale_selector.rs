use bevy_egui::egui::{self, Color32, RichText};
use ogame_core::market::CandlestickScale;

pub struct ScaleSelector<'a> {
    selected_scale: &'a mut CandlestickScale,
}

impl<'a> ScaleSelector<'a> {
    pub fn new(selected_scale: &'a mut CandlestickScale) -> Self {
        Self { selected_scale }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let button_height = 24.0;
            let text_size = 13.0;

            for (scale, label) in [
                (CandlestickScale::Minute1, "1m"),
                (CandlestickScale::Minute30, "30m"),
                (CandlestickScale::Hour1, "1h"),
                (CandlestickScale::Hour4, "4h"),
                (CandlestickScale::Day1, "1d"),
            ] {
                let selected = *self.selected_scale == scale;
                self.scale_button(ui, scale, label, selected, button_height, text_size);
            }
        });
    }

    fn scale_button(
        &mut self,
        ui: &mut egui::Ui,
        scale: CandlestickScale,
        label: &str,
        selected: bool,
        height: f32,
        text_size: f32,
    ) {
        let button = egui::Button::new(
            RichText::new(label)
                .size(text_size)
                .color(if selected {
                    Color32::from_rgb(200, 200, 255)
                } else {
                    Color32::from_rgb(160, 160, 180)
                }),
        )
        .min_size(egui::vec2(35.0, height))
        .fill(if selected {
            Color32::from_rgb(45, 45, 60)
        } else {
            Color32::from_rgb(35, 35, 45)
        });

        if ui.add(button).clicked() {
            *self.selected_scale = scale;
        }
    }
} 