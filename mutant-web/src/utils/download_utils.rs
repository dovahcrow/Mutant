// mutant-web/src/utils/download_utils.rs
use wasm_bindgen::prelude::*;
use js_sys::Uint8Array;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct JsFileHandleResult {
    #[serde(rename = "writerId")]
    pub writer_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct JsSimpleResult {
    pub error: Option<String>,
}

#[wasm_bindgen(module = "/src/utils/download.js")]
extern "C" {
    #[wasm_bindgen(js_name = initSaveFile, catch)]
    pub async fn js_init_save_file(fileName: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = writeChunk, catch)]
    pub async fn js_write_chunk(writerId: &str, chunk: Uint8Array) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = closeFile, catch)]
    pub async fn js_close_file(writerId: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = abortFile, catch)]
    pub async fn js_abort_file(writerId: &str, reason: &str) -> Result<JsValue, JsValue>;
}
