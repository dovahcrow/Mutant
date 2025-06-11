use std::sync::{Arc, Mutex};
use mutant_protocol::{KeyDetails, TaskId, TaskProgress, GetEvent};
use serde::{Deserialize, Serialize};
use log::{info, error};
use wasm_bindgen_futures::spawn_local;
use js_sys::Uint8Array;
use crate::utils::download_utils::{self, JsFileHandleResult, JsSimpleResult};

use eframe::egui;
use tokio::sync::watch; // For stop_signal

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DownloadStatus {
    PendingFilePicker,
    Initializing, // Stream started, JS writer being set up
    Downloading,
    Completed,
    Failed,
    Cancelled, // Not yet implemented, but good to have
}

#[derive(Clone)]
pub struct ActiveDownload {
    pub task_id: TaskId,
    pub file_name: String,
    pub key_details: KeyDetails,
    pub js_writer_id: Option<String>,
    pub status: DownloadStatus,
    pub progress_val: f32,
    pub downloaded_bytes: u64,
    pub error_message: Option<String>,
    pub stop_signal_tx: Option<Arc<watch::Sender<bool>>>,
    pub stop_signal_rx: Option<Arc<watch::Receiver<bool>>>,
}

impl ActiveDownload {
    pub fn new(task_id: TaskId, key_details: KeyDetails) -> Self {
        let (stop_tx, stop_rx) = watch::channel(false);
        Self {
            task_id,
            file_name: std::path::Path::new(&key_details.key)
                .file_name().map_or_else(|| key_details.key.clone(), |f| f.to_string_lossy().into_owned()),
            key_details,
            js_writer_id: None,
            status: DownloadStatus::PendingFilePicker,
            progress_val: 0.0,
            downloaded_bytes: 0,
            error_message: None,
            stop_signal_tx: Some(Arc::new(stop_tx)),
            stop_signal_rx: Some(Arc::new(stop_rx)),
        }
    }
}

pub fn update_download_state_js_writer(
    downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    task_id: TaskId,
    writer_id: String,
    new_status: DownloadStatus,
) {
    let mut downloads_guard = downloads.lock().unwrap();
    if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
        info!("Updating download {} js_writer_id to {}, status to {:?}", dl.file_name, writer_id, new_status);
        dl.js_writer_id = Some(writer_id);
        dl.status = new_status;
    }
}

pub fn update_download_progress_bytes(
    downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    task_id: TaskId,
    downloaded: u64,
) {
    let mut downloads_guard = downloads.lock().unwrap();
    if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
        dl.downloaded_bytes = downloaded;
        if dl.key_details.total_size > 0 {
            dl.progress_val = downloaded as f32 / dl.key_details.total_size as f32;
        }
    }
}

pub fn update_download_status(
    downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    task_id: TaskId,
    new_status: DownloadStatus,
    error_msg: Option<String>,
) {
    let mut downloads_guard = downloads.lock().unwrap();
    if let Some(dl) = downloads_guard.iter_mut().find(|d| d.task_id == task_id) {
        info!("Updating download {} status to {:?}, error: {:?}", dl.file_name, new_status, error_msg);
        dl.status = new_status;
        if let Some(msg) = error_msg {
            dl.error_message = Some(msg);
        }
    }
}

pub fn update_download_state_error(
    downloads: Arc<Mutex<Vec<ActiveDownload>>>,
    task_id: TaskId,
    error_message: String,
) {
    update_download_status(downloads, task_id, DownloadStatus::Failed, Some(error_message));
}

pub fn initiate_download(
    active_downloads_arc: Arc<Mutex<Vec<ActiveDownload>>>,
    key_details: KeyDetails,
    egui_ctx: egui::Context,
    // app_context is fetched directly
) {
    info!("Initiating download for key: {}", key_details.key);
    let app_context = crate::app::context::context(); // Get app context
    let key_clone = key_details.clone();
    // active_downloads_clone is active_downloads_arc, passed directly to spawned tasks if needed after cloning.

    spawn_local(async move {
        match app_context.start_streamed_get(&key_clone.key, key_clone.is_public).await {
            Ok((task_id, mut progress_receiver, mut data_receiver)) => {
                info!("start_streamed_get successful for {}. Task ID: {}", key_clone.key, task_id);
                let download_entry = ActiveDownload::new(task_id, key_clone.clone());

                let stop_rx_for_picker_failure = download_entry.stop_signal_rx.as_ref().unwrap().clone();
                let stop_tx_for_picker_failure = download_entry.stop_signal_tx.as_ref().unwrap().clone();

                active_downloads_arc.lock().unwrap().push(download_entry.clone());
                egui_ctx.request_repaint();

                let file_name_for_picker = download_entry.file_name.clone();
                match download_utils::js_init_save_file(&file_name_for_picker).await {
                    Ok(js_val) => {
                        let result: JsFileHandleResult = serde_wasm_bindgen::from_value(js_val).unwrap_or_else(|e| {
                            error!("Failed to deserialize JsFileHandleResult: {:?}", e);
                            JsFileHandleResult { writer_id: None, error: Some(format!("Deserialization error: {:?}",e)) }
                        });
                        if let Some(err_msg) = result.error {
                            error!("js_init_save_file failed for {}: {}", file_name_for_picker, err_msg);
                            let _ = stop_tx_for_picker_failure.send(true);
                            update_download_state_error(active_downloads_arc.clone(), task_id, err_msg);
                            return;
                        }
                        if let Some(writer_id) = result.writer_id {
                            info!("Obtained JS writer_id: {} for {}", writer_id, file_name_for_picker);
                            update_download_state_js_writer(active_downloads_arc.clone(), task_id, writer_id.clone(), DownloadStatus::Downloading);

                            let data_active_downloads = Arc::clone(&active_downloads_arc);
                            let data_writer_id = writer_id.clone();
                            let data_task_id = task_id;
                            let data_egui_ctx = egui_ctx.clone();
                            let data_stop_rx = stop_rx_for_picker_failure.clone();
                            spawn_local(async move {
                                let mut current_downloaded_bytes = 0;
                                let mut data_stop_rx_clone = (*data_stop_rx).clone();
                                loop {
                                    tokio::select! {
                                        changed_res = data_stop_rx_clone.changed() => {
                                            if changed_res.is_err() || *data_stop_rx.borrow() {
                                                info!("Data task for {} stopping due to signal.", data_task_id);
                                                break;
                                            }
                                        }
                                        chunk_result = data_receiver.recv() => {
                                            match chunk_result {
                                                Some(Ok(data_chunk)) => {
                                                    if data_chunk.is_empty() { continue; }
                                                    current_downloaded_bytes += data_chunk.len() as u64;
                                                    update_download_progress_bytes(data_active_downloads.clone(), data_task_id, current_downloaded_bytes);

                                                    let js_array = Uint8Array::from(&data_chunk[..]);
                                                    match download_utils::js_write_chunk(&data_writer_id, js_array).await {
                                                        Ok(val) => {
                                                            let write_result: JsSimpleResult = serde_wasm_bindgen::from_value(val).unwrap_or_else(|e| JsSimpleResult{ error: Some(format!("Deserialize error: {:?}", e))});
                                                            if let Some(err_msg) = write_result.error {
                                                                error!("js_write_chunk failed for {}: {}", data_writer_id, err_msg);
                                                                let _ = download_utils::js_abort_file(&data_writer_id, "write error").await;
                                                                update_download_state_error(data_active_downloads.clone(), data_task_id, err_msg);
                                                                break;
                                                            }
                                                        }
                                                        Err(e_write) => {
                                                            let err_msg = format!("js_write_chunk call failed: {:?}", e_write);
                                                            error!("{}", err_msg);
                                                            let _ = download_utils::js_abort_file(&data_writer_id, "write error").await;
                                                            update_download_state_error(data_active_downloads.clone(), data_task_id, err_msg);
                                                            break;
                                                        }
                                                    }
                                                    data_egui_ctx.request_repaint();
                                                }
                                                Some(Err(e)) => {
                                                    let err_msg = format!("Data stream error: {}", e);
                                                    error!("{}", err_msg);
                                                    let _ = download_utils::js_abort_file(&data_writer_id, "stream error").await;
                                                    update_download_state_error(data_active_downloads.clone(), data_task_id, err_msg);
                                                    break;
                                                }
                                                None => {
                                                    info!("Data stream ended for {}. Closing file.", data_task_id);
                                                    match download_utils::js_close_file(&data_writer_id).await {
                                                        Ok(val) => {
                                                            let close_result: JsSimpleResult = serde_wasm_bindgen::from_value(val).unwrap_or_else(|e| JsSimpleResult{ error: Some(format!("Deserialize error: {:?}", e))});
                                                            if let Some(err_msg) = close_result.error {
                                                                error!("js_close_file failed for {}: {}", data_writer_id, err_msg);
                                                                update_download_state_error(data_active_downloads.clone(), data_task_id, err_msg);
                                                            }
                                                        }
                                                        Err(e_close) => {
                                                             let err_msg = format!("js_close_file call failed: {:?}", e_close);
                                                             error!("{}", err_msg);
                                                             update_download_state_error(data_active_downloads.clone(), data_task_id, err_msg);
                                                        }
                                                    }
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            });

                            let prog_active_downloads = Arc::clone(&active_downloads_arc);
                            let prog_task_id = task_id;
                            let prog_egui_ctx = egui_ctx.clone();
                            let prog_stop_rx = stop_rx_for_picker_failure.clone();
                            spawn_local(async move {
                                let mut prog_stop_rx_clone = (*prog_stop_rx).clone();
                                 loop {
                                    tokio::select! {
                                        changed_res = prog_stop_rx_clone.changed() => {
                                             if changed_res.is_err() || *prog_stop_rx.borrow() {
                                                info!("Progress task for {} stopping due to signal.", prog_task_id);
                                                break;
                                            }
                                        }
                                        progress_event = progress_receiver.recv() => {
                                            match progress_event {
                                                Some(Ok(TaskProgress::Get(get_event))) => {
                                                    match get_event {
                                                        GetEvent::Starting { total_chunks } => {
                                                            info!("Download Starting for {}: {} total chunks.", prog_task_id, total_chunks);
                                                        }
                                                        GetEvent::PadData { .. } => { }
                                                        GetEvent::Complete => {
                                                            info!("Download Complete event for {}", prog_task_id);
                                                            update_download_status(prog_active_downloads.clone(), prog_task_id, DownloadStatus::Completed, None);
                                                            prog_egui_ctx.request_repaint();
                                                            break;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                Some(Err(e)) => {
                                                    let err_msg = format!("Progress stream error: {}", e);
                                                    error!("{}", err_msg);
                                                    update_download_state_error(prog_active_downloads.clone(), prog_task_id, err_msg);
                                                    prog_egui_ctx.request_repaint();
                                                    break;
                                                }
                                                None => {
                                                    info!("Progress stream ended for {}", prog_task_id);
                                                    break;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            });
                        } else {
                            let err_msg = "js_init_save_file returned no writerId and no error.".to_string();
                            error!("{}", err_msg);
                            let _ = stop_tx_for_picker_failure.send(true);
                            update_download_state_error(active_downloads_arc.clone(), task_id, err_msg);
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("js_init_save_file call failed: {:?}", e);
                        error!("{}", err_msg);
                        let _ = stop_tx_for_picker_failure.send(true);
                        update_download_state_error(active_downloads_arc.clone(), task_id, err_msg);
                    }
                }
            }
            Err(e) => {
                error!("start_streamed_get failed: {}", e);
                // No task_id here, so can't update a specific download entry.
                // This case should be rare, implies a fundamental issue with starting the stream.
            }
        }
        egui_ctx.request_repaint();
    });
}
