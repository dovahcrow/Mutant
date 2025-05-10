use bevy_egui::egui::{self, Color32, Layout, RichText};
use ogame_core::{market::OrderSide, protocol::Protocol};

use crate::utils::game_mut;

pub struct OrderList<'a> {
    selected_item: &'a str,
    height: f32,
}

impl<'a> OrderList<'a> {
    pub fn new(selected_item: &'a str, height: f32) -> Self {
        Self {
            selected_item,
            height,
        }
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        let game = crate::game();
        let orders: Vec<ogame_core::market::Order> = game
            .orders
            .values()
            .filter(|order| order.item_id == self.selected_item)
            .cloned()
            .collect();
        let current_user_id = game.user_id.clone();
        drop(game);

        let (sells, buys): (Vec<_>, Vec<_>) = orders
            .into_iter()
            .partition(|order| order.side == OrderSide::Sell);

        let available_width = ui.available_width();
        let column_width = available_width / 2.0;

        ui.with_layout(Layout::left_to_right(egui::Align::TOP), |ui| {
            // Sell orders table
            ui.allocate_ui_with_layout(
                egui::vec2(column_width, ui.available_height()),
                Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.label(
                        RichText::new("Sell Orders")
                            .color(Color32::from_rgb(255, 100, 100))
                            .heading(),
                    );
                    egui::ScrollArea::vertical()
                        .id_source("sell_scroll")
                        .max_height(self.height)
                        .show(ui, |ui| {
                            self.draw_order_list(
                                ui,
                                sells,
                                Color32::from_rgb(255, 100, 100),
                                &current_user_id,
                                "sell",
                                true,
                            );
                        });
                },
            );

            ui.separator();

            // Buy orders table
            ui.allocate_ui_with_layout(
                egui::vec2(column_width, ui.available_height()),
                Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.label(
                        RichText::new("Buy Orders")
                            .color(Color32::from_rgb(100, 255, 100))
                            .heading(),
                    );
                    egui::ScrollArea::vertical()
                        .id_source("buy_scroll")
                        .max_height(self.height)
                        .show(ui, |ui| {
                            self.draw_order_list(
                                ui,
                                buys,
                                Color32::from_rgb(100, 255, 100),
                                &current_user_id,
                                "buy",
                                false,
                            );
                        });
                },
            );
        });
    }

    fn draw_order_list(
        &self,
        ui: &mut egui::Ui,
        mut orders: Vec<ogame_core::market::Order>,
        color: Color32,
        current_user_id: &str,
        id_prefix: &str,
        ascending: bool,
    ) {
        if ascending {
            orders.sort_by(|a, b| a.price.cmp(&b.price));
        } else {
            orders.sort_by(|a, b| b.price.cmp(&a.price));
        }

        let available_width = ui.available_width();
        let col_width = (available_width - 40.0) / 3.0; // Subtract some padding and divide by number of columns

        egui::Grid::new(format!("{}_orders_grid", id_prefix))
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                // Headers
                ui.scope(|ui| {
                    ui.set_min_width(col_width);
                    ui.label(RichText::new("Price").strong());
                });
                ui.scope(|ui| {
                    ui.set_min_width(col_width);
                    ui.label(RichText::new("Volume").strong());
                });
                ui.scope(|ui| {
                    ui.set_min_width(col_width);
                    ui.label(RichText::new("").strong());
                });
                ui.end_row();

                // Orders
                for order in orders {
                    let price_text = format!("{:.2}", order.price as f64 / 100.0);
                    let volume_text = format!("{}", order.volume);

                    ui.scope(|ui| {
                        ui.set_min_width(col_width);
                        ui.label(RichText::new(price_text).color(color));
                    });
                    ui.scope(|ui| {
                        ui.set_min_width(col_width);
                        ui.label(volume_text);
                    });
                    ui.scope(|ui| {
                        ui.set_min_width(col_width);
                        if order.user_id == current_user_id {
                            if ui.button("❌").clicked() {
                                let _ =
                                    game_mut().action(Protocol::CancelOrder { order_id: order.id });
                            }
                        } else {
                            ui.label(""); // Empty cell for non-owned orders
                        }
                    });
                    ui.end_row();
                }
            });
    }
}
