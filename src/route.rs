//! Deciding which engine reads a file, before any work is done.
//!
//! Routing is separated from reading on purpose. A caller can ask what *would*
//! happen — which engine, how many pages, how many of them need OCR — in a few
//! milliseconds, and only then decide whether to spend the money.

use crate::error::{ReadError, Result};

/// Which engine handles this file, and what it will cost.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// Word, Excel, PowerPoint, OpenDocument, RTF, EPUB, CSV. Read by `anydoc`.
    Office(anydoc::Format),
    /// A PDF. Read by `pdf-inspector`, page by page.
    Pdf(PdfPlan),
    /// A photograph or scan. Read by the OCR engine in [`crate::ocr`].
    Image,
    /// Nothing here recognises it.
    Unknown,
}

/// What a PDF actually contains.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPlan {
    pub page_count: u32,
    /// 1-indexed pages with no usable text layer.
    pub pages_needing_ocr: Vec<u32>,
    pub kind: PdfKind,
    /// Detection confidence, 0.0 to 1.0.
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfKind {
    /// Every page has text. No OCR needed.
    Text,
    /// Some pages have text and some do not. This is the case the other two
    /// libraries fail on, and the reason this one exists.
    Mixed,
    /// No page has text.
    Scanned,
}

/// What reading this file will involve.
#[derive(Debug, Clone)]
pub struct Plan {
    pub route: Route,
    /// True when at least one page cannot be read without OCR.
    pub needs_ocr: bool,
    /// How many pages need OCR. Zero for office files and text PDFs.
    pub ocr_page_count: usize,
    pub inspect_time_ms: u64,
}

impl Plan {
    /// A one-line description a human or a log can read.
    pub fn summary(&self) -> String {
        match &self.route {
            Route::Office(f) => format!("{f:?} document, no OCR needed"),
            Route::Pdf(p) => match p.kind {
                PdfKind::Text => format!("text PDF, {} pages, no OCR needed", p.page_count),
                PdfKind::Mixed => format!(
                    "mixed PDF, {} pages, {} need OCR",
                    p.page_count,
                    p.pages_needing_ocr.len()
                ),
                PdfKind::Scanned => {
                    format!("scanned PDF, all {} pages need OCR", p.page_count)
                }
            },
            Route::Image => "image, needs OCR".to_string(),
            Route::Unknown => "unrecognised".to_string(),
        }
    }
}

/// Work out how a file would be read, without reading it.
pub fn inspect(bytes: &[u8]) -> Result<Plan> {
    let watch = crate::clock::Stopwatch::start();
    let route = classify(bytes)?;
    let (needs_ocr, ocr_page_count) = match &route {
        Route::Office(_) => (false, 0),
        Route::Pdf(p) => (!p.pages_needing_ocr.is_empty(), p.pages_needing_ocr.len()),
        Route::Image => (true, 1),
        Route::Unknown => (false, 0),
    };
    Ok(Plan {
        route,
        needs_ocr,
        ocr_page_count,
        inspect_time_ms: watch.elapsed_ms(),
    })
}

fn classify(bytes: &[u8]) -> Result<Route> {
    if bytes.is_empty() {
        return Err(ReadError::Unsupported("the file is empty".into()));
    }

    // anydoc reads the format from the bytes. PDFs are handed on to
    // pdf-inspector so we can see inside them page by page.
    if let Some(format) = anydoc::Format::from_bytes(bytes) {
        if format != anydoc::Format::Pdf {
            return Ok(Route::Office(format));
        }
        return Ok(Route::Pdf(inspect_pdf(bytes)?));
    }

    if is_image(bytes) {
        return Ok(Route::Image);
    }

    Ok(Route::Unknown)
}

fn inspect_pdf(bytes: &[u8]) -> Result<PdfPlan> {
    let detected = pdf_inspector::detect_pdf_mem(bytes)?;
    let page_count = detected.page_count;
    let pages_needing_ocr = detected.pages_needing_ocr.clone();

    let kind = if pages_needing_ocr.is_empty() {
        PdfKind::Text
    } else if pages_needing_ocr.len() as u32 >= page_count {
        PdfKind::Scanned
    } else {
        PdfKind::Mixed
    };

    Ok(PdfPlan {
        page_count,
        pages_needing_ocr,
        kind,
        confidence: detected.confidence,
    })
}

/// Recognise an image by its own signature. `anydoc` has no image formats, so
/// this is the boundary between "a document" and "a picture of one".
fn is_image(bytes: &[u8]) -> bool {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.format())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let img = image::GrayImage::from_pixel(20, 20, image::Luma([255u8]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn a_png_routes_to_ocr() {
        let plan = inspect(&png()).unwrap();
        assert_eq!(plan.route, Route::Image);
        assert!(plan.needs_ocr);
        assert_eq!(plan.summary(), "image, needs OCR");
    }

    #[test]
    fn an_empty_file_is_rejected() {
        assert!(matches!(inspect(&[]), Err(ReadError::Unsupported(_))));
    }

    #[test]
    fn random_bytes_are_unknown_not_an_error() {
        let plan = inspect(b"just some prose, not a document at all").unwrap();
        assert_eq!(plan.route, Route::Unknown);
        assert!(!plan.needs_ocr);
    }

    #[test]
    fn inspection_is_fast() {
        let plan = inspect(&png()).unwrap();
        assert!(
            plan.inspect_time_ms < 200,
            "took {}ms",
            plan.inspect_time_ms
        );
    }
}
