use bevy_egui::egui::{self, Color32, Frame, RichText, ScrollArea, Stroke, Vec2};
use ogame_core::{
    market::{Candlestick, CandlestickScale},
    GAME_DATA,
};
use std::collections::BTreeMap;

pub struct ItemSelector<'a> {
    selected_item: &'a mut String,
    candlesticks: &'a BTreeMap<
        String,
        BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
    >,
}

impl<'a> ItemSelector<'a> {
    pub fn new(
        selected_item: &'a mut String,
        candlesticks: &'a BTreeMap<
            String,
            BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
        >,
    ) -> Self {
        Self {
            selected_item,
            candlesticks,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.set_height(ui.available_height());

            // Header
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.heading(
                    RichText::new("Market Items")
                        .size(18.0)
                        .color(Color32::WHITE),
                );
                ui.add_space(8.0);
            });

            // Items list in a frame taking remaining height
            let frame = Frame::none()
                .fill(Color32::from_rgb(20, 20, 25))
                .stroke(Stroke::new(1.0, Color32::from_rgb(40, 40, 45)))
                .inner_margin(4.0)
                .rounding(egui::Rounding::same(4.0));

            frame.show(ui, |ui| {
                // Calculate remaining height after header
                let remaining_height = ui.available_height() - 40.0;
                ScrollArea::vertical()
                    .max_height(remaining_height)
                    .show(ui, |ui| {
                        self.show_items_grid(ui);
                    });
            });
        });
    }

    fn show_items_grid(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("items_grid")
            .spacing(Vec2::new(8.0, 4.0))
            .show(ui, |ui| {
                let binding = GAME_DATA.read();
                let items: Vec<_> = binding.recipes.values().cloned().collect();

                // Get market data safely
                let market_data = self.candlesticks.values().next();

                for (idx, item) in items.iter().enumerate() {
                    let (current_price, price_change_pct) = if let Some(market_data) = market_data {
                        self.get_price_data(market_data, &item.name)
                    } else {
                        (0, 0.0)
                    };

                    self.show_item_row(ui, &item.name, current_price, price_change_pct);
                    ui.end_row();
                }
            });
    }

    fn show_item_row(
        &mut self,
        ui: &mut egui::Ui,
        item_name: &str,
        current_price: i32,
        price_change_pct: f64,
    ) {
        let selected = *self.selected_item == item_name;
        let color = if price_change_pct >= 0.0 {
            Color32::from_rgb(100, 255, 100)
        } else {
            Color32::from_rgb(255, 100, 100)
        };

        // Item name
        let text = RichText::new(item_name)
            .color(if selected {
                Color32::WHITE
            } else {
                Color32::from_rgb(180, 180, 180)
            })
            .size(14.0);

        if ui.selectable_label(selected, text).clicked() {
            *self.selected_item = item_name.to_string();
        }

        // Price
        ui.label(
            RichText::new(format!("{:.2}", current_price as f64 / 100.0))
                .color(Color32::WHITE)
                .size(14.0),
        );

        // Change percentage
        ui.label(
            RichText::new(format!("{:+.2}%", price_change_pct))
                .color(color)
                .size(14.0),
        );
    }

    fn get_price_data(
        &self,
        market_data: &BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
        item_name: &str,
    ) -> (i32, f64) {
        market_data
            .get(item_name)
            .and_then(|scales| {
                let minute_data = scales.get(&CandlestickScale::Minute1);
                let day_data = scales.get(&CandlestickScale::Day1);

                minute_data.and_then(|candles| {
                    day_data.and_then(|day_candles| {
                        let current = candles.values().last()?;
                        let day_first = day_candles.values().last()?;

                        let price_change = current.close as f64 - day_first.open as f64;
                        let change_pct = (price_change / day_first.open as f64) * 100.0;

                        Some((current.close as i32, change_pct))
                    })
                })
            })
            .unwrap_or((0, 0.0))
    }
}
