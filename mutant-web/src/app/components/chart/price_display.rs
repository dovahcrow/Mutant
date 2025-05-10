use bevy_egui::egui::{self, Color32, RichText};
use ogame_core::market::{Candlestick, CandlestickScale};
use std::collections::BTreeMap;

pub struct PriceDisplay<'a> {
    candlesticks: &'a BTreeMap<
        String,
        BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
    >,
    selected_item: &'a str,
    selected_scale: &'a CandlestickScale,
}

impl<'a> PriceDisplay<'a> {
    pub fn new(
        candlesticks: &'a BTreeMap<
            String,
            BTreeMap<String, BTreeMap<CandlestickScale, BTreeMap<String, Candlestick>>>,
        >,
        selected_item: &'a str,
        selected_scale: &'a CandlestickScale,
    ) -> Self {
        Self {
            candlesticks,
            selected_item,
            selected_scale,
        }
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            // Title row with item name
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(self.selected_item)
                        .color(Color32::WHITE)
                        .size(18.0)
                        .strong(),
                );
            });

            // Stats grid
            egui::Grid::new("price_stats")
                .spacing([40.0, 8.0])
                .show(ui, |ui| {
                    if let Some((first, last)) = self.get_first_and_last_candlestick() {
                        let price_change = last.close as f64 - first.open as f64;
                        let price_change_pct = (price_change / first.open as f64) * 100.0;
                        let color = if price_change >= 0.0 {
                            Color32::from_rgb(100, 255, 100)
                        } else {
                            Color32::from_rgb(255, 100, 100)
                        };

                        // First row
                        ui.label(RichText::new("Selling").color(Color32::from_rgb(180, 180, 180)));
                        ui.label(
                            RichText::new(format!("{:.2}", last.close as f64 / 100.0))
                                .color(Color32::WHITE)
                                .size(16.0),
                        );
                        ui.label(
                            RichText::new(format!("{:+.2}%", price_change_pct))
                                .color(color)
                                .size(16.0),
                        );
                        ui.end_row();

                        // Second row
                        ui.label(RichText::new("Buying").color(Color32::from_rgb(180, 180, 180)));
                        ui.label(
                            RichText::new(format!("{:.2}", (last.close as f64 * 0.95) / 100.0))
                                .color(Color32::WHITE)
                                .size(16.0),
                        );
                        ui.label(
                            RichText::new(format!("Volume: {}", self.get_total_volume()))
                                .color(Color32::from_rgb(180, 180, 180))
                                .size(14.0),
                        );
                        ui.end_row();
                    } else {
                        // No market data available
                        ui.label(RichText::new("Selling").color(Color32::from_rgb(180, 180, 180)));
                        ui.label(RichText::new("0").color(Color32::WHITE).size(16.0));
                        ui.label(
                            RichText::new("0.00%")
                                .color(Color32::from_rgb(180, 180, 180))
                                .size(16.0),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Buying").color(Color32::from_rgb(180, 180, 180)));
                        ui.label(RichText::new("0").color(Color32::WHITE).size(16.0));
                        ui.label(
                            RichText::new("Volume: 0")
                                .color(Color32::from_rgb(180, 180, 180))
                                .size(14.0),
                        );
                        ui.end_row();
                    }
                });
        });
    }

    fn get_first_and_last_candlestick(&self) -> Option<(&Candlestick, &Candlestick)> {
        let candlesticks = self
            .candlesticks
            .values()
            .next()?
            .get(self.selected_item)?
            .get(self.selected_scale)?;

        Some((candlesticks.values().next()?, candlesticks.values().last()?))
    }

    fn get_total_volume(&self) -> u64 {
        let empty = BTreeMap::new();
        let empty2 = BTreeMap::new();
        let market_data = self.candlesticks.values().next().unwrap_or(&empty);
        let item_data = market_data.get(self.selected_item).unwrap_or(&empty2);

        item_data
            .get(self.selected_scale)
            .map(|data| data.values().map(|c| c.volume).sum())
            .unwrap_or(0)
    }
}
