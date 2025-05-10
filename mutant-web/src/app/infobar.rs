use bevy::prelude::Resource;
use bevy_egui::egui::{
    self, Color32, Context, FontFamily, FontId, Margin, Rounding, Stroke, Vec2b,
};

use super::window_system_mut;

#[derive(Resource)]
pub struct InfobarWindow {}

impl InfobarWindow {
    pub fn new() -> Self {
        Self {}
    }

    pub fn render(&mut self, ctx: &mut Context) {
        let browser_window = web_sys::window().unwrap();

        // Set up modern styling for the entire context
        self.setup_context_style(ctx);

        // Create the frame style that will be used for both windows
        let frame = self.create_frame_style();

        // Render the left infobar
        self.render_infobar(
            ctx,
            "Left Infobar",
            frame.clone(),
            egui::Align2::LEFT_TOP,
            browser_window.inner_height().unwrap().as_f64().unwrap() as f32,
        );
    }

    fn setup_context_style(&self, ctx: &mut Context) {
        ctx.style_mut(|style| {
            // set background color to #1b1b1b
            style.visuals.window_fill = Color32::from_rgba_premultiplied(27, 27, 27, 200);
            style.visuals.window_rounding = Rounding::same(1.0);
            style.visuals.window_stroke = Stroke {
                width: 1.0,
                color: Color32::from_rgba_premultiplied(100, 100, 120, 140),
            };

            // Set monospace font for all text styles
            for (_key, font) in style.text_styles.iter_mut() {
                font.family = FontFamily::Monospace;
            }

            style
                .text_styles
                .get_mut(&egui::TextStyle::Body)
                .unwrap()
                .size = 14.0;
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.window_margin = Margin::same(12.0);

            style.visuals.striped = true;
        });
    }

    fn create_frame_style(&self) -> egui::Frame {
        egui::Frame::default()
            .fill(Color32::from_rgba_premultiplied(15, 15, 20, 200))
            .inner_margin(Margin::same(2.0))
            .rounding(Rounding::same(1.0))
            .stroke(Stroke::new(
                1.0,
                Color32::from_rgba_premultiplied(100, 100, 120, 140),
            ))
            .shadow(egui::epaint::Shadow {
                offset: egui::Vec2::ZERO,
                blur: 8.0,
                spread: 0.0,
                color: Color32::from_black_alpha(40),
            })
    }

    fn render_infobar(
        &mut self,
        ctx: &mut Context,
        title: &str,
        frame: egui::Frame,
        anchor: egui::Align2,
        window_height: f32,
    ) {
        egui::Window::new(title)
            .resizable([true, false])
            .frame(frame)
            .constrain(true)
            .collapsible(false)
            .title_bar(false)
            .scroll(Vec2b::TRUE)
            .default_size(egui::Vec2::new(300.0, 0.0))
            .min_size(egui::Vec2::new(10.0, window_height))
            .anchor(anchor, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                self.setup_ui_style(ui);
                window_system_mut().draw(ui);
            });
    }

    fn setup_ui_style(&self, ui: &mut egui::Ui) {
        ui.style_mut().visuals.widgets.noninteractive.bg_fill =
            Color32::from_rgba_premultiplied(15, 15, 20, 200);
        ui.style_mut().visuals.widgets.inactive.bg_fill =
            Color32::from_rgba_premultiplied(15, 15, 20, 200);
        ui.style_mut().visuals.widgets.hovered.bg_fill =
            Color32::from_rgba_premultiplied(25, 25, 35, 200);
        ui.style_mut().visuals.widgets.active.bg_fill =
            Color32::from_rgba_premultiplied(35, 35, 45, 220);

        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        ui.spacing_mut().window_margin = Margin::same(1.0);
    }
}
