use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, RichText};
use mutant_protocol::{Task, TaskListEntry, TaskProgress, TaskResult, TaskStatus};
use serde::{Deserialize, Serialize};

use super::Window;

#[derive(Clone, Serialize, Deserialize)]
pub struct TasksWindow {
    tasks: Arc<RwLock<Option<Vec<TaskListEntry>>>>,
    selected_task: Arc<RwLock<Option<Task>>>,
    connected: bool,
}

impl Default for TasksWindow {
    fn default() -> Self {
        Self {
            tasks: crate::app::context::context().get_task_cache(),
            selected_task: Arc::new(RwLock::new(None)),
            connected: false,
        }
    }
}

impl Window for TasksWindow {
    fn name(&self) -> String {
        "MutAnt Tasks".to_string()
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.draw_tasks_list(ui);
    }
}

impl TasksWindow {
    pub fn new() -> Self {
        let mut tasks_window = Self::default();

        // Fetch tasks when window is created
        let tasks = tasks_window.tasks.clone();
        let connected_ref = Arc::new(RwLock::new(false));
        let connected = connected_ref.clone();
        let self_connected = Arc::new(RwLock::new(false));
        let connected_clone = self_connected.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let ctx = crate::app::context::context();
            let (task_list, is_connected) = ctx._list_tasks().await;
            // *tasks.write().unwrap() = task_list;
            *connected.write().unwrap() = is_connected;
            *connected_clone.write().unwrap() = is_connected;
        });

        // Set connected status
        tasks_window.connected = *self_connected.read().unwrap();

        tasks_window
    }

    pub fn draw_tasks_list(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new("MutAnt Tasks")
                .size(24.0)
                .color(Color32::from_rgb(100, 200, 255)),
        );

        // Connection status
        ui.horizontal(|ui| {
            let (status_text, status_color) = if self.connected {
                ("Connected", Color32::from_rgb(100, 255, 100))
            } else {
                ("Disconnected", Color32::from_rgb(255, 100, 100))
            };

            ui.label("Status: ");
            ui.label(RichText::new(status_text).color(status_color));
        });

        ui.add_space(12.0);

        // Tasks list
        if self.tasks.read().unwrap().is_none() {
            ui.label(RichText::new("No active tasks.").color(Color32::GRAY));
            crate::app::context::context()._list_tasks();
            return;
        }
        if let Some(tasks) = &*self.tasks.read().unwrap() {
            if tasks.is_empty() {
                ui.label(RichText::new("No active tasks.").color(Color32::GRAY));
            } else {
                // Create a table to display the tasks
                egui::Grid::new("tasks_grid")
                    .num_columns(3)
                    .spacing([20.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // Header row
                        ui.strong(RichText::new("Task ID").color(Color32::LIGHT_BLUE));
                        ui.strong(RichText::new("Type").color(Color32::LIGHT_BLUE));
                        ui.strong(RichText::new("Status").color(Color32::LIGHT_BLUE));
                        ui.end_row();

                        // Data rows
                        for task in tasks {
                            // Task ID (shortened)
                            let task_id_str = task.task_id.to_string();
                            let short_id = if task_id_str.len() > 8 {
                                format!("{}...", &task_id_str[0..8])
                            } else {
                                task_id_str.clone()
                            };

                            if ui.label(short_id).clicked() {
                                let task_id = task.task_id;
                                let selected_task = self.selected_task.clone();

                                wasm_bindgen_futures::spawn_local(async move {
                                    let ctx = crate::app::context::context();
                                    if let Ok(task_details) = ctx.get_task(task_id).await {
                                        *selected_task.write().unwrap() = Some(task_details);
                                    }
                                });
                            }

                            // Task type
                            ui.label(format!("{:?}", task.task_type));

                            // Status with color
                            let (status_text, status_color) = match task.status {
                                TaskStatus::Completed => {
                                    ("Completed", Color32::from_rgb(100, 255, 100))
                                }
                                TaskStatus::Failed => ("Failed", Color32::from_rgb(255, 100, 100)),
                                TaskStatus::InProgress => {
                                    ("In Progress", Color32::from_rgb(255, 200, 0))
                                }
                                TaskStatus::Pending => {
                                    ("Pending", Color32::from_rgb(200, 200, 200))
                                }
                                TaskStatus::Stopped => {
                                    ("Stopped", Color32::from_rgb(150, 150, 150))
                                }
                            };

                            ui.label(RichText::new(status_text).color(status_color));
                            ui.end_row();
                        }
                    });
            }
        }

        // Display selected task details if any
        if let Some(task) = &*self.selected_task.read().unwrap() {
            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading(
                RichText::new("Task Details")
                    .size(18.0)
                    .color(Color32::LIGHT_BLUE),
            );

            ui.horizontal(|ui| {
                ui.label("ID: ");
                ui.label(RichText::new(task.id.to_string()).monospace());
            });

            ui.horizontal(|ui| {
                ui.label("Type: ");
                ui.label(RichText::new(format!("{:?}", task.task_type)).strong());
            });

            ui.horizontal(|ui| {
                ui.label("Status: ");
                let (status_text, status_color) = match task.status {
                    TaskStatus::Completed => ("Completed", Color32::from_rgb(100, 255, 100)),
                    TaskStatus::Failed => ("Failed", Color32::from_rgb(255, 100, 100)),
                    TaskStatus::InProgress => ("In Progress", Color32::from_rgb(255, 200, 0)),
                    TaskStatus::Pending => ("Pending", Color32::from_rgb(200, 200, 200)),
                    TaskStatus::Stopped => ("Stopped", Color32::from_rgb(150, 150, 150)),
                };
                ui.label(RichText::new(status_text).color(status_color));
            });

            if let Some(key) = &task.key {
                ui.horizontal(|ui| {
                    ui.label("Key: ");
                    ui.label(key);
                });
            }

            // Progress information
            if let Some(progress) = &task.progress {
                ui.add_space(5.0);
                ui.label(RichText::new("Progress:").strong());

                match progress {
                    TaskProgress::Put(event) => {
                        ui.label(format!("Put Event: {:?}", event));
                    }
                    TaskProgress::Get(event) => {
                        ui.label(format!("Get Event: {:?}", event));
                    }
                    TaskProgress::Sync(event) => {
                        ui.label(format!("Sync Event: {:?}", event));
                    }
                    TaskProgress::Purge(event) => {
                        ui.label(format!("Purge Event: {:?}", event));
                    }
                    TaskProgress::HealthCheck(event) => {
                        ui.label(format!("Health Check Event: {:?}", event));
                    }
                }
            }

            // Result information
            ui.add_space(5.0);
            ui.label(RichText::new("Result:").strong());

            match &task.result {
                TaskResult::Pending => {
                    ui.label(RichText::new("Pending").color(Color32::YELLOW));
                }
                TaskResult::Error(error) => {
                    ui.label(RichText::new(format!("Error: {}", error)).color(Color32::RED));
                }
                TaskResult::Result(result) => {
                    ui.label(
                        RichText::new(format!("Completed: {:?}", result)).color(Color32::GREEN),
                    );
                }
            }

            // Stop button for in-progress tasks
            if task.status == TaskStatus::InProgress || task.status == TaskStatus::Pending {
                ui.add_space(10.0);
                if ui.button("Stop Task").clicked() {
                    let task_id = task.id;
                    let selected_task = self.selected_task.clone();
                    let tasks_list = self.tasks.clone();

                    wasm_bindgen_futures::spawn_local(async move {
                        let ctx = crate::app::context::context();
                        if ctx.stop_task(task_id).await.is_ok() {
                            // Invalidate caches to ensure fresh data
                            ctx.invalidate_caches();

                            // Refresh task list and details
                            let (task_list, _) = ctx._list_tasks().await;

                            if let Ok(updated_task) = ctx.get_task(task_id).await {
                                *selected_task.write().unwrap() = Some(updated_task);
                            }
                        }
                    });
                }
            }
        }

        ui.add_space(12.0);

        // Refresh and Reconnect buttons
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                let tasks = self.tasks.clone();
                let connected_ref = Arc::new(RwLock::new(false));
                let connected = connected_ref.clone();
                let selected_task = self.selected_task.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    let ctx = crate::app::context::context();

                    // Invalidate caches to ensure fresh data
                    ctx.invalidate_caches();

                    // Get fresh task list
                    let (task_list, is_connected) = ctx._list_tasks().await;

                    // Refresh selected task if there is one
                    if let Some(task) = &*selected_task.read().unwrap() {
                        if let Ok(updated_task) = ctx.get_task(task.id).await {
                            *selected_task.write().unwrap() = Some(updated_task);
                        }
                    }
                });

                // Update the connected flag after spawning the task
                self.connected = *connected_ref.read().unwrap();
            }

            if ui.button("Reconnect").clicked() {
                log::info!("Reconnect clicked");
                let tasks = self.tasks.clone();
                let selected_task = self.selected_task.clone();
                let connected_ref = Arc::new(RwLock::new(false));
                let connected = connected_ref.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    let ctx = crate::app::context::context();

                    // Force a reconnection
                    if let Err(e) = ctx.reconnect().await {
                        log::error!("Failed to reconnect: {}", e);
                    }

                    // Refresh the tasks list
                    let (task_list, is_connected) = ctx.list_tasks().await;
                    // *tasks.write().unwrap() = task_list;
                    // *connected.write().unwrap() = is_connected;

                    // Refresh selected task if there is one
                    if let Some(task) = &*selected_task.read().unwrap() {
                        if let Ok(updated_task) = ctx.get_task(task.id).await {
                            *selected_task.write().unwrap() = Some(updated_task);
                        }
                    }
                });

                // Update the connected flag after spawning the task
                self.connected = *connected_ref.read().unwrap();
            }
        });
    }
}
