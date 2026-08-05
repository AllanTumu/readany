//! WebAssembly bindings for readany.
//!
//! The whole free surface of the library runs here: office documents, PDFs
//! that carry a text layer, file inspection, and the image correction stages.
//! Text recognition needs a native runtime and is not part of this build,
//! which is why a page with no text layer comes back named rather than
//! silently dropped.

use readany::{Origin, Route};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// What reading a file will involve, worked out without reading it.
#[derive(Serialize)]
pub struct Plan {
    /// "office", "pdf", "image" or "unknown".
    pub kind: String,
    /// A sentence a person can read.
    pub summary: String,
    pub needs_ocr: bool,
    /// How many pages carry no text layer.
    pub ocr_page_count: usize,
    pub inspect_time_ms: f64,
}

/// The result of reading a file.
#[derive(Serialize)]
pub struct Document {
    pub markdown: String,
    pub pages: usize,
    /// Pages read from the file's own text layer.
    pub extracted: usize,
    /// 1-indexed pages that carry no text and were not read.
    pub unresolved_pages: Vec<u32>,
    /// True when every page that needed reading was read.
    pub complete: bool,
    /// A one-line account of what happened, for a log or an audit trail.
    pub receipt: String,
    pub processing_time_ms: f64,
}

/// Work out what a file is and what reading it will cost. Reads nothing.
#[wasm_bindgen]
pub fn inspect(bytes: &[u8]) -> Result<JsValue, JsError> {
    let plan = readany::inspect(bytes).map_err(to_js)?;
    let kind = match &plan.route {
        Route::Office(_) => "office",
        Route::Pdf(_) => "pdf",
        Route::Image => "image",
        Route::Unknown => "unknown",
    };
    let out = Plan {
        kind: kind.to_string(),
        summary: plan.summary(),
        needs_ocr: plan.needs_ocr,
        ocr_page_count: plan.ocr_page_count,
        inspect_time_ms: plan.inspect_time_ms as f64,
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}

/// Read a file into Markdown, with a full account of every page.
#[wasm_bindgen]
pub fn read(bytes: &[u8]) -> Result<JsValue, JsError> {
    let doc = readany::read(bytes).map_err(to_js)?;
    let extracted = doc
        .pages
        .iter()
        .filter(|p| p.origin == Origin::Text)
        .count();
    let out = Document {
        markdown: doc.markdown.clone(),
        pages: doc.pages.len(),
        extracted,
        unresolved_pages: doc.unresolved_pages.clone(),
        complete: doc.is_complete(),
        receipt: doc.receipt(),
        processing_time_ms: doc.processing_time_ms as f64,
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}

/// Read a file and return only the Markdown.
#[wasm_bindgen(js_name = toMarkdown)]
pub fn to_markdown(bytes: &[u8]) -> Result<String, JsError> {
    readany::to_markdown(bytes).map_err(to_js)
}

fn to_js(e: readany::ReadError) -> JsError {
    JsError::new(&e.to_string())
}
