mod candlestick_chart;
mod item_selector;
mod order_list;
mod price_display;
mod scale_selector;
mod utils;

use bevy_egui::egui::{self, Response, SidePanel};
use ogame_core::market::CandlestickScale;
use serde::{Deserialize, Serialize};

pub use self::candlestick_chart::CandlestickChart;
pub use self::item_selector::ItemSelector;
pub use self::order_list::OrderList;
pub use self::price_display::PriceDisplay;
pub use self::scale_selector::ScaleSelector;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Chart {
    selected_scale: CandlestickScale,
    selected_item: String,
    #[serde(default = "default_orders_height")]
    orders_height: f32,
}

fn default_orders_height() -> f32 {
    200.0
}

impl Chart {
    pub fn new() -> Self {
        Self {
            selected_scale: CandlestickScale::Minute1,
            selected_item: "iron".to_string(),
            orders_height: default_orders_height(),
        }
    }

    pub fn box_plot(&mut self, ui: &mut egui::Ui) -> Response {
        let candlesticks = crate::game().candlesticks.clone();
        let available_height = ui.available_height();

        ui.horizontal_centered(|ui| {
            ui.set_height(available_height);

            // Left sidebar with item selector
            SidePanel::left("item_selector_panel")
                .resizable(true)
                .min_width(200.0)
                .default_width(250.0)
                .show_inside(ui, |ui| {
                    ui.set_height(available_height);
                    ItemSelector::new(&mut self.selected_item, &candlesticks).show(ui);
                });

            ui.vertical(|ui| {
                ui.set_height(available_height);

                // Top section with fixed height for price display and scale selector
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 80.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        // Price display
                        PriceDisplay::new(&candlesticks, &self.selected_item, &self.selected_scale)
                            .show(ui);

                        ui.add_space(4.0);

                        // Scale selector
                        ui.horizontal(|ui| {
                            ScaleSelector::new(&mut self.selected_scale).show(ui);
                        });
                    },
                );

                ui.add_space(4.0);

                let content_height = ui.available_height();

                egui::containers::Frame::none().show(ui, |ui| {
                    egui::TopBottomPanel::bottom("orders_panel")
                        .resizable(true)
                        .min_height(100.0)
                        .default_height(self.orders_height)
                        .height_range(100.0..=content_height - 100.0)
                        .show_inside(ui, |ui| {
                            OrderList::new(&self.selected_item, ui.available_height()).show(ui);
                        });

                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        CandlestickChart::new(
                            &candlesticks,
                            &self.selected_item,
                            &self.selected_scale,
                            ui.available_height(),
                        )
                        .show(ui);
                    });
                });
            })
            .response
        })
        .inner
    }
}
