use serde::{Deserialize, Serialize};
use eframe::egui;
use egui_dock;
use crate::app::fs::viewer_tab::FileViewerTab;
use crate::app::Window;
use crate::app::put::PutWindow;
use crate::app::stats::StatsWindow;
use crate::app::theme;
use crate::app::fs::tree::TreeNode;
use log;

/// Unified tab type for the FsWindow's internal dock system
#[derive(Clone, Serialize, Deserialize)]
pub enum FsInternalTab {
    FileViewer(FileViewerTab),
    Put(PutWindow),
    Stats(StatsWindow),
}

impl FsInternalTab {
    pub fn name(&self) -> String {
        match self {
            Self::FileViewer(tab) => {
                let file_name = std::path::Path::new(&tab.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| tab.file.key.clone());
                file_name
            }
            Self::Put(window) => window.name(),
            Self::Stats(window) => window.name(),
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        match self {
            Self::FileViewer(tab) => tab.draw(ui),
            Self::Put(window) => window.draw(ui),
            Self::Stats(window) => window.draw(ui),
        }
    }
}

/// TabViewer for FsInternalTab in the FsWindow's internal dock
pub struct FsInternalTabViewer {}

impl egui_dock::TabViewer for FsInternalTabViewer {
    type Tab = FsInternalTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            FsInternalTab::FileViewer(file_tab) => {
                let file_name = std::path::Path::new(&file_tab.file.key)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_tab.file.key.clone());

                let (file_icon, _icon_color) = {
                    let temp_node = TreeNode::new_file(&file_name, file_tab.file.clone());
                    temp_node.get_file_icon_and_color()
                };

                let modified_indicator = if file_tab.file_modified { "* " } else { "" };
                let title = format!("{}{}{}", file_icon, modified_indicator, file_name);
                egui::RichText::new(title).size(12.0).into()
            }
            FsInternalTab::Put(_) => egui::RichText::new("📤 Upload").size(12.0).into(),
            FsInternalTab::Stats(_) => egui::RichText::new("📊 Stats").size(12.0).into(),
        }
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> bool {
        match tab {
            FsInternalTab::FileViewer(file_tab) => {
                if file_tab.file_modified {
                    log::info!("Closing modified file: {}", file_tab.file.key);
                }
                // Cleanup video player if it exists
                file_tab.cleanup_video_player();
                true
            }
            _ => true,
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let frame = egui::Frame::new()
            .fill(theme::MutantColors::BACKGROUND_DARK)
            .inner_margin(egui::Margin::same(8));

        frame.show(ui, |ui| {
            tab.draw(ui);
        });
    }

    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }
}
