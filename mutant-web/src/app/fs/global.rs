use std::sync::{Arc, RwLock};
use lazy_static::lazy_static;
use crate::app::fs_window::FsWindow; // Corrected path

lazy_static! {
    static ref MAIN_FS_WINDOW: Arc<RwLock<Option<Arc<RwLock<FsWindow>>>>> = Arc::new(RwLock::new(None));
}

/// Set the global reference to the main FsWindow
pub fn set_main_fs_window(fs_window: Arc<RwLock<FsWindow>>) {
    *MAIN_FS_WINDOW.write().unwrap() = Some(fs_window);
}

/// Get a reference to the main FsWindow for async tasks
pub fn get_main_fs_window() -> Option<Arc<RwLock<FsWindow>>> {
    MAIN_FS_WINDOW.read().unwrap().clone()
}
