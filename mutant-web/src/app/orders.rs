use bevy_egui::egui::{self, Color32, Grid, RichText, Vec2};
use egui_dock::egui::{text::LayoutJob, Ui};
use ogame_core::{
    market::{Order, OrderSide},
    protocol::Protocol,
    GAME_DATA,
};

use crate::{game, game_mut};

use super::Window;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct OrdersWindow {
    market_id: String,
    item_id: String,
    side: String,
    price: String,
    volume: String,
}

impl OrdersWindow {
    pub fn new() -> Self {
        Self {
            market_id: "".to_string(),
            item_id: "iron".to_string(),
            side: "buy".to_string(),
            price: "".to_string(),
            volume: "".to_string(),
        }
    }
}

impl Window for OrdersWindow {
    fn name(&self) -> String {
        format!("Market - {}", self.item_id)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading("Market Orders");
        });
        ui.add_space(8.0);

        self.draw_order_form(ui);
        ui.add_space(16.0);
        self.draw_orderbook(ui);
    }
}

impl OrdersWindow {
    fn draw_order_form(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let content_width = available_width - 32.0;
        let label_width = 60.0;

        egui::Frame::none()
            .stroke(ui.style().visuals.widgets.noninteractive.bg_stroke)
            .rounding(8.0)
            .fill(Color32::from_rgb(25, 25, 30))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(content_width);
                    ui.set_max_width(content_width);

                    // Header with background
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(RichText::new("New Order").size(18.0));
                    });
                    ui.add_space(12.0);

                    // Item selector with fixed label width
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.scope(|ui| {
                            ui.set_min_width(label_width);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center)
                                    .with_cross_align(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new("Item:").size(14.0).strong());
                                },
                            );
                        });
                        egui::ComboBox::from_id_salt("order_item")
                            .selected_text(RichText::new(&self.item_id).size(14.0))
                            .width(content_width - label_width - 24.0)
                            .show_ui(ui, |ui| {
                                for item in GAME_DATA.read().recipes.values() {
                                    ui.selectable_value(
                                        &mut self.item_id,
                                        item.name.clone(),
                                        RichText::new(&item.name).size(14.0),
                                    );
                                }
                            });
                    });

                    ui.add_space(12.0);

                    // Form grid with fixed label width
                    Grid::new("order_form_grid")
                        .num_columns(2)
                        .spacing([8.0, 12.0])
                        .min_col_width(label_width)
                        .show(ui, |ui| {
                            // Order type
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center)
                                    .with_cross_align(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new("Type:").size(14.0).strong());
                                },
                            );
                            ui.horizontal(|ui| {
                                let button_size =
                                    Vec2::new((content_width - label_width - 24.0) / 2.0, 28.0);
                                if ui
                                    .add_sized(
                                        button_size,
                                        egui::SelectableLabel::new(
                                            self.side == "buy",
                                            RichText::new("Buy")
                                                .size(14.0)
                                                .color(Color32::from_rgb(0, 255, 0)),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.side = "buy".to_string();
                                }
                                if ui
                                    .add_sized(
                                        button_size,
                                        egui::SelectableLabel::new(
                                            self.side == "sell",
                                            RichText::new("Sell")
                                                .size(14.0)
                                                .color(Color32::from_rgb(255, 0, 0)),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.side = "sell".to_string();
                                }
                            });
                            ui.end_row();

                            // Volume input
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center)
                                    .with_cross_align(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new("Volume:").size(14.0).strong());
                                },
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.volume)
                                    .hint_text("Amount to trade")
                                    .desired_width(content_width - label_width - 24.0),
                            );
                            ui.end_row();

                            // Price input
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center)
                                    .with_cross_align(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new("Price:").size(14.0).strong());
                                },
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.price)
                                    .hint_text("Price per unit")
                                    .desired_width(content_width - label_width - 24.0),
                            );
                            ui.end_row();
                        });

                    ui.add_space(16.0);

                    // Submit button with better styling
                    let can_submit = !self.volume.is_empty()
                        && !self.price.is_empty()
                        && self.volume.parse::<u32>().is_ok()
                        && self.price.parse::<f64>().is_ok();

                    ui.vertical_centered(|ui| {
                        ui.add_enabled_ui(can_submit, |ui| {
                            let button_text = match self.side.as_str() {
                                "buy" => "🔵 Place Buy Order",
                                "sell" => "🔴 Place Sell Order",
                                _ => "Place Order",
                            };

                            if ui
                                .add_sized(
                                    Vec2::new(200.0, 32.0),
                                    egui::Button::new(
                                        RichText::new(button_text).size(14.0).strong(),
                                    ),
                                )
                                .clicked()
                            {
                                self.submit_order();
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
            });
    }

    fn draw_orderbook(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("Order Book").small());
            ui.add_space(4.0);
        });

        let available_height = ui.available_height() - 50.0;
        let available_width = ui.available_width();
        let column_width = (available_width - 20.0) / 2.0;

        // Get all data we need at once
        let game = game();
        let orders: Vec<Order> = game
            .orders
            .values()
            .filter(|order| order.item_id == self.item_id)
            .cloned()
            .collect();
        let current_user_id = game.user_id.clone();
        drop(game); // Release the lock before UI work

        if orders.is_empty() {
            ui.label("No active orders");
            return;
        }

        let (sells, buys): (Vec<Order>, Vec<_>) = orders
            .into_iter()
            .partition(|order| order.side == OrderSide::Sell);

        Grid::new("orderbook_grid")
            .num_columns(2)
            .spacing([20.0, 4.0])
            .min_col_width(column_width)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(column_width);
                    ui.set_max_width(column_width);
                    ui.label(RichText::new("Sell Orders").color(Color32::from_rgb(255, 0, 0)));
                    self.draw_order_list(
                        ui,
                        sells,
                        Color32::from_rgb(255, 0, 0),
                        &current_user_id,
                        available_height,
                        column_width,
                    );
                });

                ui.vertical(|ui| {
                    ui.set_min_width(column_width);
                    ui.set_max_width(column_width);
                    ui.label(RichText::new("Buy Orders").color(Color32::from_rgb(0, 255, 0)));
                    self.draw_order_list(
                        ui,
                        buys,
                        Color32::from_rgb(0, 255, 0),
                        &current_user_id,
                        available_height,
                        column_width,
                    );
                });
            });
    }

    fn draw_order_list(
        &self,
        ui: &mut Ui,
        mut orders: Vec<Order>,
        color: Color32,
        current_user_id: &str,
        available_height: f32,
        width: f32,
    ) {
        orders.sort_by(|a, b| b.price.cmp(&a.price));

        egui::ScrollArea::vertical()
            .id_salt(format!(
                "orders_scroll_{}_{}_side",
                self.item_id,
                if color == Color32::from_rgb(0, 255, 0) {
                    "buy"
                } else {
                    "sell"
                }
            ))
            .max_height(available_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);
                for order in orders {
                    let is_own_order = order.user_id == current_user_id;
                    let row_color = if is_own_order {
                        color.linear_multiply(0.3)
                    } else {
                        Color32::TRANSPARENT
                    };

                    egui::Frame::none().fill(row_color).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} @ {:.2}",
                                    order.volume,
                                    order.price as f64 / 100.0
                                ))
                                .color(color),
                            );

                            if is_own_order {
                                if ui.small_button("❌").clicked() {
                                    let _ = game_mut().action(Protocol::CancelOrder {
                                        order_id: order.id.clone(),
                                    });
                                }
                            }
                        });
                    });
                }
            });
    }

    fn submit_order(&mut self) {
        if let (Ok(volume), Ok(price)) = (self.volume.parse::<u32>(), self.price.parse::<f64>()) {
            let price_as_int = (price * 100.0) as usize;
            let _ = game_mut().action(Protocol::NewOrder {
                market_id: self.market_id.clone(),
                item_id: self.item_id.clone(),
                side: match self.side.as_str() {
                    "buy" => OrderSide::Buy,
                    "sell" => OrderSide::Sell,
                    _ => OrderSide::Buy,
                },
                price: price_as_int,
                volume: volume.try_into().unwrap(),
            });

            // Clear form
            self.price.clear();
            self.volume.clear();
        }
    }
}
