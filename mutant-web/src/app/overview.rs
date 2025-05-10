use std::collections::BTreeMap;

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32, RichText, Ui};
use ogame_core::{base::Base, research::Research, BaseId};

use crate::game;

use super::{base::BaseWindow, components::progress::progress, ship::ShipWindow, Window};

use serde::{Deserialize, Serialize};
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct OverviewWindow {
    toggle_open: Option<bool>,

    base_windows: BTreeMap<BaseId, BaseWindow>,
}

impl Default for OverviewWindow {
    fn default() -> Self {
        let base_windows = game()
            .bases
            .clone()
            .into_iter()
            .map(|(id, base)| (id, BaseWindow::new(base.id)))
            .collect();

        Self {
            toggle_open: Some(true),
            base_windows,
        }
    }
}

impl Window for OverviewWindow {
    fn name(&self) -> String {
        "Overview".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.all(ui);
    }
}

impl OverviewWindow {
    fn all(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .button(
                    RichText::new("📖 Open all")
                        .strong()
                        .color(Color32::from_rgb(100, 200, 100)),
                )
                .clicked()
            {
                self.toggle_open = Some(true);
            }
            if ui
                .button(
                    RichText::new("📕 Close all")
                        .strong()
                        .color(Color32::from_rgb(200, 100, 100)),
                )
                .clicked()
            {
                self.toggle_open = Some(false);
            }
        });

        ui.add_space(8.0);

        ui.vertical(|ui| {
            self.technologies(ui);
            ui.add_space(4.0);
            self.ships(ui);
            ui.add_space(4.0);
            self.bases(ui);
        });

        self.toggle_open = None;
    }

    fn technologies(&mut self, ui: &mut egui::Ui) {
        let technologies = game().technologies.clone();

        egui::CollapsingHeader::new(
            RichText::new("🧪 Technologies")
                .heading()
                .strong()
                .color(Color32::from_rgb(100, 200, 255)),
        )
        .open(self.toggle_open)
        .show(ui, |ui| {
            // Current Research section
            egui::CollapsingHeader::new(
                RichText::new("Current Research")
                    .strong()
                    .color(Color32::from_rgb(255, 200, 100)),
            )
            .open(self.toggle_open)
            .show(ui, |ui| {
                let research = game().research.clone();
                if let Some(research) = research {
                    self.draw_current_research(research, ui);
                } else {
                    ui.label(RichText::new("No active research").color(Color32::GRAY));
                }
            });

            // Researched Technologies section
            egui::CollapsingHeader::new(
                RichText::new("Researched Technologies")
                    .strong()
                    .color(Color32::from_rgb(255, 200, 100)),
            )
            .open(self.toggle_open)
            .show(ui, |ui| {
                if technologies.is_empty() {
                    ui.label(RichText::new("No technologies researched yet").color(Color32::GRAY));
                } else {
                    for technology in technologies {
                        ui.label(
                            RichText::new(format!("✓ {}", technology))
                                .color(Color32::from_rgb(100, 255, 150)),
                        );
                    }
                }
            });
        });
    }

    fn draw_current_research(&self, research: Research, ui: &mut Ui) {
        let completion = research.completion();
        ui.vertical(|ui| {
            ui.label(RichText::new(&research.name).strong());
            ui.add(progress(completion, research.get_formated_duration()));
        });
    }

    fn ships(&mut self, ui: &mut egui::Ui) {
        let ships = game().ships.clone();

        egui::CollapsingHeader::new(
            RichText::new("🚀 Ships")
                .heading()
                .strong()
                .color(Color32::from_rgb(255, 150, 150)),
        )
        .open(self.toggle_open)
        .show(ui, |ui| {
            if ships.is_empty() {
                ui.label(RichText::new("No ships available").color(Color32::GRAY));
            } else {
                for (ship_id, _ship) in ships {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("Ship {}", ship_id))
                            .strong()
                            .color(Color32::from_rgb(255, 180, 180)),
                    )
                    .open(self.toggle_open)
                    .show(ui, |ui| {
                        ShipWindow::new(ship_id.clone()).ship_infos("OverviewShips", ui);
                    });
                }
            }
        });
    }

    fn bases(&mut self, ui: &mut egui::Ui) {
        let bases = game().bases.clone();

        egui::CollapsingHeader::new(
            RichText::new("🌍 Bases")
                .heading()
                .strong()
                .color(Color32::from_rgb(150, 255, 150)),
        )
        .open(self.toggle_open)
        .show(ui, |ui| {
            if bases.is_empty() {
                ui.label(RichText::new("No bases established").color(Color32::GRAY));
            } else {
                for (_base_id, base) in bases {
                    self.base(base, ui);
                }
            }
        });
    }

    fn draw_section_header(&mut self, title: &str, ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
        egui::CollapsingHeader::new(RichText::new(title).heading().strong())
            .open(self.toggle_open)
            .show(ui, content);
    }

    fn draw_subsection(&mut self, title: &str, ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .open(self.toggle_open)
            .show(ui, content);
    }

    fn base(&mut self, base: Base, ui: &mut egui::Ui) {
        let base_window = self
            .base_windows
            .entry(base.id.clone())
            .or_insert_with(|| BaseWindow::new(base.id.clone()));

        base_window.base_infos_overview(format!("OverviewBase {}", base.id), ui);
    }
}
