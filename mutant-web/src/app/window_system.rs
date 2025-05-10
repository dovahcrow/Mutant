use std::sync::{Arc, MappedRwLockWriteGuard, RwLock, RwLockWriteGuard};

use eframe::egui::{self, Id, RichText, SidePanel};
use egui_dock::{DockArea, DockState, Style};
use futures::{SinkExt, StreamExt};
use lazy_static::lazy_static;

use super::{main::MainWindow, Window, WindowType};


lazy_static! {
    static ref WINDOW_SYSTEM: Arc<RwLock<WindowSystem>> =
        Arc::new(RwLock::new(WindowSystem::default()));
}

pub fn window_system_mut() -> RwLockWriteGuard<'static, WindowSystem> {
    WINDOW_SYSTEM.write().unwrap()
}

lazy_static::lazy_static! {
    static ref NEW_WINDOW_TX: RwLock<Option<futures::channel::mpsc::Sender<WindowType>>>
        = RwLock::new(None);
}

fn new_window_tx() -> MappedRwLockWriteGuard<'static, futures::channel::mpsc::Sender<WindowType>> {
    RwLockWriteGuard::map(NEW_WINDOW_TX.write().unwrap(), |lol| match lol {
        Some(x) => x,
        None => panic!("Game not initialized"),
    })
}

pub fn new_window<T: Into<WindowType> + 'static>(window: T) {
    wasm_bindgen_futures::spawn_local(async move {
        new_window_tx().send(window.into()).await.unwrap();
    });
}




struct TabViewer {}

impl egui_dock::TabViewer for TabViewer {
    type Tab = WindowType;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        (&*tab.name()).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        // ui.label(format!("Content of {tab}"));
        tab.draw(ui);
    }
}

pub struct WindowSystem {
    tree: DockState<WindowType>,
    need_focus: Option<(i32, i32)>,
    // chart_window: ChartWindow,
    frame: usize,
}

impl WindowSystem {
    pub fn add_window(&mut self, window: WindowType) {
        if self
            .tree
            .iter_all_tabs()
            .any(|((_, _), w)| w.name() == window.name())
        {
            return;
        }

        let size = Self::default_window_size(&window);

        let surface = self.tree.add_window(vec![window]);

        self.tree
            .get_window_state_mut(surface)
            .unwrap()
            .set_size(size.into());
    }

    fn default_window_size(window_type: &WindowType) -> [f32; 2] {
        match window_type {
            // WindowType::Chart(_) => [1000.0, 300.0],
            // WindowType::Overview(_) => [300.0, 600.0],
            // WindowType::Ships(_) => [300.0, 600.0],
            // WindowType::Bases(_) => [600.0, 400.0],
            // WindowType::Research(_) => [600.0, 400.0],
            // WindowType::Flights(_) => [600.0, 500.0],
            // WindowType::Orders(_) => [300.0, 400.0],
            // WindowType::Trades(_) => [300.0, 400.0],
            // WindowType::Blueprints(_) => [300.0, 500.0],
            _ => [300.0, 400.0],
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // Change from SidePanel::right to SidePanel::left
        SidePanel::left("left_menu")
            .exact_width(25.0)
            .show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);

                    // Helper function to check if window is open
                    let is_window_open = |name: &str| {
                        self.tree
                            .iter_all_tabs()
                            .any(|((_, _), w)| w.name() == name)
                    };

                    // Helper function to create a button with different styling when active
                    let menu_button =
                        |ui: &mut egui::Ui, icon: &str, hover: &str, is_open: bool| {
                            let button = egui::Button::new(if is_open {
                                RichText::new(icon)
                                    .strong()
                                    .background_color(ui.style().visuals.selection.bg_fill)
                            } else {
                                RichText::new(icon)
                            });
                            ui.add(button).on_hover_text(hover)
                        };

                    if menu_button(ui, "🛸", "Main", is_window_open("Main")).clicked() {
                        new_window(MainWindow::new());
                    }

                    // if menu_button(ui, "🚀", "Ships", is_window_open("Ships")).clicked() {
                    //     new_window(ShipsWindow {});
                    // }

                    // if menu_button(ui, "🏠", "Bases", is_window_open("Bases")).clicked() {
                    //     new_window(BasesWindow {});
                    // }

                    // if menu_button(ui, "📊", "Overview", is_window_open("Overview")).clicked() {
                    //     new_window(OverviewWindow::default());
                    // }

                    // if menu_button(ui, "🔬", "Research", is_window_open("Research")).clicked() {
                    //     new_window(ResearchWindow {
                    //         selected_base: None,
                    //     });
                    // }

                    // if menu_button(ui, "📋", "Orders", is_window_open("Orders")).clicked() {
                    //     new_window(OrdersWindow::new())
                    // }

                    // if menu_button(ui, "💱", "Trades", is_window_open("Trades")).clicked() {
                    //     new_window(TradesWindow::new());
                    // }

                    // if menu_button(ui, "📈", "Chart", is_window_open("Chart")).clicked() {
                    //     self.chart_window.id += 1;
                    //     new_window(self.chart_window.clone());
                    // }

                    // if menu_button(ui, "📝", "Blueprint", is_window_open("Blueprint")).clicked() {
                    //     new_window(BlueprintsWindow::new());
                    // }

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |_ui| {
                        // if ui.button("🚪").on_hover_text("Logout").clicked() {
                        //     // SessionCookie::logout();
                        //     let window = web_sys::window().unwrap();
                        //     window.location().reload().unwrap();
                        // }
                    });
                });
            });

        DockArea::new(&mut self.tree)
            .style(Style::from_egui(ui.ctx().style().as_ref()))
            .id(Id::new("egui_dock::DockArea"))
            .show_inside(ui, &mut TabViewer {});

        self.frame += 1;

        if self.frame >= 10 {
            let window = web_sys::window().unwrap();
            let serie = serde_json::to_string(&self.tree).unwrap();
            // let serie = serie.iter().map(|x| *x as char).collect::<String>();
            window
                .local_storage()
                .unwrap()
                .unwrap()
                .set(
                    "egui_dock::DockArea",
                    &serie,
                )
                .unwrap();

            self.frame = 0;
        }
    }


    pub fn from_memory(user_id: String) -> Self {
        let window = web_sys::window().unwrap();
        let data = window
            .local_storage()
            .unwrap()
            .unwrap()
            .get(format!("egui_dock::DockArea{}", user_id).as_str())
            .unwrap();

        let tree = if let Some(data) = data {
            let tree = serde_json::from_str(&data)
                .unwrap_or_else(|_e| DockState::new(vec![]));
            tree
        } else {
            DockState::new(vec![MainWindow::default().into()])
        };
        Self {
            tree,
            ..Default::default()
        }
    }
}

impl Default for WindowSystem {
    fn default() -> Self {
        let tree = DockState::new(vec![]);

        Self {
            frame: 0,
            tree,
            need_focus: None,
        }
    }
}

pub async fn init_window_system() {
    let (new_window_tx, mut new_window_rx) = futures::channel::mpsc::channel(1);
    // let (request_focus_tx, mut request_focus_rx) = futures::channel::mpsc::channel(1);

    *NEW_WINDOW_TX.write().unwrap() = Some(new_window_tx);
    // *REQUEST_FOCUS_TX.write().unwrap() = Some(request_focus_tx);

    wasm_bindgen_futures::spawn_local(async move {
        while let Some(window) = new_window_rx.next().await {
            window_system_mut().add_window(window);
        }
    });

    // wasm_bindgen_futures::spawn_local(async move {
    //     while let Some(pos) = request_focus_rx.next().await {
    //         window_system_mut().request_focus(pos);
    //     }
    // });
}

// pub fn focus_first_base() {
//     let game = game();

//     if let Some(base) = game.bases.iter().next() {
//         request_focus(base.1);

//         return;
//     } else if let Some(ship) = game.ships.iter().next() {
//         let Some(entity_id) = &ship.1.position_id else {
//             return;
//         };

//         let entity = game.get_entity(&entity_id).unwrap();

//         request_focus(&entity);

//         return;
//     }

//     GAME_DATA.read().planets.iter().next().map(|(_, planet)| {
//         request_focus(planet);
//     });
// }
