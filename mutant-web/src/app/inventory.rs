use bevy_egui::egui::{self, Color32, Frame, Id, RichText, Sense, Stroke, Vec2};
use ogame_core::{protocol::Protocol, StorageId, GAME_DATA};
use std::sync::Arc;

use crate::{game, game_mut};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Location {
    pub storage_id: StorageId,
    pub item: (String, usize),
}

const SLOT_SIZE: f32 = 50.0;
const GRID_SPACING: f32 = 0.0;

pub fn inventory<I: Into<Id>>(storage_id: StorageId, salt_id: I, ui: &mut egui::Ui) {
    let storage = game().storages.get(&storage_id).unwrap().clone();
    let game_data = GAME_DATA.read();

    // Add storage usage bars
    let (weight_pct, volume_pct) = storage.get_usage_percentages();
    let (total_weight, total_volume) = storage.get_usage();
    
    ui.vertical(|ui| {
        // Weight progress bar
        ui.horizontal(|ui| {
            ui.label("Weight:");
            let weight_bar = egui::ProgressBar::new(weight_pct)
                .text(format!("{:.1}kg / {}kg",
                    total_weight as f32 / 1000.0,  // Convert g to kg
                    storage.max_weight as f32 / 1000.0));
            ui.add(weight_bar);
        });

        // Volume progress bar
        ui.horizontal(|ui| {
            ui.label("Volume:");
            let volume_bar = egui::ProgressBar::new(volume_pct)
                .text(format!("{:.1}m³ / {}m³",
                    total_volume as f32 / 1000000.0,  // Convert cm³ to m³
                    storage.max_volume as f32 / 1000000.0));
            ui.add(volume_bar);
        });
    });

    let frame = Frame::default()
        .inner_margin(1.0)
        .fill(Color32::from_rgb(20, 20, 25))
        .rounding(4.0)
        .stroke(Stroke::new(1.0, Color32::from_gray(60)));

    let salt = salt_id.into();

    let (response, dropped_payload) = ui.dnd_drop_zone::<Location, ()>(frame, |ui| {
        if storage.items.is_empty() {
            // Show a single empty slot
            let slot_frame = Frame::none()
                .fill(Color32::from_rgb(25, 25, 30))
                .rounding(2.0)
                .stroke(Stroke::new(1.0, Color32::from_rgb(40, 40, 45)));

            ui.add_sized(
                Vec2::splat(SLOT_SIZE),
                egui::Button::new("").fill(slot_frame.fill),
            );
        } else {
            let available_width = ui.available_width();
            let num_columns = ((available_width / SLOT_SIZE).floor() as usize).min(5);
            let num_columns = num_columns.max(1);

            let grid = egui::Grid::new(format!("Grid {}", salt.value()))
                .spacing([1.0, 1.0])
                .num_columns(num_columns);

            grid.show(ui, |ui| {
                for (i, (item_id, amount)) in storage.items.iter().enumerate() {
                    let item_data = game_data.recipes.get(item_id);
                    let item_gui_id = Id::new((salt.value(), i, storage_id.clone(), item_id));

                    let slot_frame = Frame::none()
                        .fill(Color32::from_rgb(30, 30, 35))
                        .rounding(2.0)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(40, 40, 45)));

                    let response = ui
                        .dnd_drag_source(
                            item_gui_id,
                            Location {
                                storage_id: storage_id.clone(),
                                item: (item_id.clone(), *amount),
                            },
                            |ui| {
                                ui.add_sized(
                                    Vec2::splat(SLOT_SIZE),
                                    egui::Button::new("")
                                        .fill(slot_frame.fill)
                                        .sense(Sense::click_and_drag()),
                                );
                            },
                        )
                        .response;

                    let painter = ui.painter_at(response.rect);
                    draw_item_slot(
                        &painter,
                        response.rect,
                        item_id,
                        *amount,
                        &item_data,
                        response.hovered(),
                    );

                    handle_hover_and_drop(ui, &response, &storage_id, item_id, *amount);

                    if i % num_columns == (num_columns - 1) {
                        ui.end_row();
                    }
                }

                let remaining = (num_columns - (storage.items.len() % num_columns)) % num_columns;
                for _ in 0..remaining {
                    let slot_frame = Frame::none()
                        .fill(Color32::from_rgb(25, 25, 30))
                        .rounding(2.0)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(40, 40, 45)));

                    ui.add_sized(
                        Vec2::splat(SLOT_SIZE),
                        egui::Button::new("").fill(slot_frame.fill),
                    );
                }
            });
        }
    });

    handle_dropped_payload(dropped_payload, storage_id);
}

fn draw_item_contents(
    ui: &mut egui::Ui,
    item_id: &str,
    amount: usize,
    item_data: &Option<&ogame_core::Recipe>,
) {
    let text = RichText::new(format!("{}\n{}", item_id, amount))
        .color(Color32::WHITE)
        .size(14.0);
    ui.label(text);
}

fn draw_item_slot(
    painter: &egui::Painter,
    rect: egui::Rect,
    item_id: &str,
    amount: usize,
    _item_data: &Option<&ogame_core::Recipe>,
    hovered: bool,
) {
    if hovered {
        painter.rect_filled(rect, 2.0, Color32::from_gray(60));
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(100)));
    }

    // Draw item name in center
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        item_id,
        egui::FontId::proportional(10.0),
        Color32::WHITE,
    );

    // Draw amount in bottom right with padding
    painter.text(
        rect.right_bottom() - Vec2::new(4.0, 4.0),
        egui::Align2::RIGHT_BOTTOM,
        amount.to_string(),
        egui::FontId::proportional(9.0),
        Color32::from_gray(200),
    );
}

fn handle_hover_and_drop(
    ui: &mut egui::Ui,
    response: &egui::Response,
    storage_id: &StorageId,
    item_id: &str,
    amount: usize,
) {
    if let (Some(pointer), Some(hovered_payload)) = (
        ui.input(|i| i.pointer.interact_pos()),
        response.dnd_hover_payload::<Location>(),
    ) {
        let rect = response.rect;
        let stroke = Stroke::new(1.0, Color32::WHITE);

        if *hovered_payload
            == (Location {
                storage_id: storage_id.clone(),
                item: (item_id.to_string(), amount),
            })
        {
            ui.painter().hline(rect.x_range(), rect.center().y, stroke);
        } else if pointer.y < rect.center().y {
            ui.painter().hline(rect.x_range(), rect.top(), stroke);
        } else {
            ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
        }

        if let Some(dragged_payload) = response.dnd_release_payload() {
            handle_item_move(
                dragged_payload,
                storage_id.clone(),
                item_id.to_string(),
                amount,
            );
        }
    }
}

fn handle_dropped_payload(dropped_payload: Option<Arc<Location>>, target_storage: StorageId) {
    if let Some(from) = dropped_payload {
        if let Err(e) = game_mut().action(Protocol::MoveItems {
            origin: from.storage_id.clone(),
            target: target_storage,
            recipe_id: from.item.0.clone(),
            amount: from.item.1,
        }) {
            log::error!("Failed to move items: {:?}", e);
        }
    }
}

fn handle_item_move(
    from: Arc<Location>,
    to_storage: StorageId,
    _to_item: String,
    _to_amount: usize,
) {
    if let Err(e) = game_mut().action(Protocol::MoveItems {
        origin: from.storage_id.clone(),
        target: to_storage,
        recipe_id: from.item.0.clone(),
        amount: from.item.1,
    }) {
        log::error!("Failed to move items: {:?}", e);
    }
}
