//! PDF page handling: deciding how to read a page, and mapping between the
//! two coordinate spaces involved when a page has to be rasterised.
//!
//! No implementation of [`render::Rasterise`] lives here or anywhere in this
//! crate. `readany` ships to npm as WebAssembly and PDFium is native C++, so
//! the renderer lives in `readany-pdfium` — the same split already used for
//! OCR, where [`crate::OcrBackend`] is declared here and implemented in
//! `readany-ocr`.

pub mod coords;
pub mod decide;
pub mod render;
