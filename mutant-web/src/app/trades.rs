use bevy_egui::egui::{self, Color32};
use egui_dock::egui::{text::LayoutJob, FontId, Label, Layout};
use ogame_core::GAME_DATA;

use crate::game;

use super::Window;

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct TradesWindow {
    selected_item: String,
}

impl TradesWindow {
    pub fn new() -> Self {
        Self {
            selected_item: "iron".to_string(),
        }
    }
}

impl Window for TradesWindow {
    fn name(&self) -> String {
        format!("Trades {}", self.selected_item)
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let trades = game().trades.clone();

        egui::ComboBox::from_id_salt("trades_item")
            .selected_text(format!("{:?}", self.selected_item))
            .show_ui(ui, |ui| {
                for item in GAME_DATA.read().recipes.values() {
                    ui.selectable_value(&mut self.selected_item, item.name.clone(), &item.name);
                }
            });

        let mut trades = trades
            .into_iter()
            .filter(|(_, trade)| trade.item_id == self.selected_item)
            .collect::<Vec<_>>();

        trades.sort_by_key(|(_, trade)| -trade.timestamp);

        let mut last_price = 0;
        let mut last_color = Color32::from_rgb(0, 255, 0);

        let trades = trades
            .iter()
            .map(|(_, trade)| {
                let color = if trade.price < last_price {
                    Color32::from_rgb(255, 0, 0)
                } else if trade.price > last_price {
                    Color32::from_rgb(0, 255, 0)
                } else {
                    last_color.clone()
                };
                last_color = color;
                last_price = trade.price;
                (trade, color)
            })
            .rev()
            .collect::<Vec<_>>();

        let layout = Layout::default().with_cross_align(egui::Align::RIGHT);
        /* .heading()
        .text_color(Color32::from_rgb(255, 255, 255))
        .h1(); */

        egui::Grid::new(format!("trades_{}", self.selected_item))
            .num_columns(3)
            .spacing([40.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(layout_text_heading("Volume"));

                ui.label(layout_text_heading("Price"));

                ui.label(layout_text_heading("Date"));

                ui.end_row();

                trades.into_iter().rev().for_each(|(trade, color)| {
                    ui.with_layout(layout, |ui| {
                        ui.label(layout_text(&trade.volume.to_string()));
                    });
                    ui.with_layout(layout, |ui| {
                        ui.label(layout_text_with_color(&trade.price.to_string(), color));
                    });
                    ui.label(format!(
                        " ({})",
                        chrono::DateTime::from_timestamp(trade.timestamp as i64, 0)
                            .unwrap()
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string(),
                    ));
                    ui.end_row();
                });
            });
    }
}

fn layout_text_heading(text: &str) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: FontId::proportional(15.0),
            underline: egui::Stroke::new(1.0, Color32::WHITE),
            valign: egui::Align::RIGHT,
            ..Default::default()
        },
    );

    job.halign = egui::Align::RIGHT;

    job
}

fn layout_text(text: &str) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: FontId::proportional(12.0),
            valign: egui::Align::RIGHT,
            ..Default::default()
        },
    );

    job.halign = egui::Align::RIGHT;

    job
}

fn layout_text_with_color(text: &str, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            color,
            valign: egui::Align::RIGHT,
            font_id: FontId::proportional(12.0),
            ..Default::default()
        },
    );

    job.halign = egui::Align::RIGHT;

    job
}
