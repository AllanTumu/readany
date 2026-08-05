//! OCR: reading pages that carry no text, only pixels.
//!
//! This is the third engine. [`crate`] routes typed documents to `anydoc` and
//! typed PDFs to `pdf-inspector`; anything that is only an image lands here.

pub mod detect;
pub mod engine;
pub mod error;
pub mod image;
pub mod layout;
pub mod markdown;
pub mod models;
pub mod recognize;
pub mod types;

pub use engine::{prepare, prepare_bytes, Engine, Prepared, ScanOptions};
pub use error::{Result, ScanError};
pub use markdown::MarkdownOptions;
pub use types::{Quad, ScanResult, TextBox, TextLine};

/// Anything that can read an image into text.
///
/// The full [`Engine`] implements this, but so could a call to a remote
/// service or a stub in a test. Keeping it a trait means the router never
/// depends on how OCR happens, and a build with no OCR at all still works.
pub trait OcrBackend {
    fn read_image(&self, bytes: &[u8]) -> Result<ScanResult>;
}

impl<D: detect::Detector, R: recognize::Recognizer> OcrBackend for Engine<D, R> {
    fn read_image(&self, bytes: &[u8]) -> Result<ScanResult> {
        self.scan_bytes(bytes)
    }
}
