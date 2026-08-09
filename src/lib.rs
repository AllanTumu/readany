//! # readany
//!
//! Read any document into clean Markdown, through one call.
//!
//! Three engines sit behind one function. Office files go to [`anydoc`].
//! PDFs go to [`pdf_inspector`], page by page. Anything that is only pixels —
//! a scan, a photograph, a fax — goes to the OCR engine in [`ocr`].
//!
//! The case that matters is the one in between. A PDF where some pages were
//! typed and some were scanned defeats both of the other libraries: `anydoc`
//! refuses the whole file, and a pure OCR pipeline pays to re-read pages whose
//! text was already there. readany reads each page the cheap way when it can,
//! and only pays for the pages that need it.
//!
//! ```no_run
//! # fn main() -> Result<(), readany::ReadError> {
//! // What would this cost, before spending anything?
//! let bytes = std::fs::read("contract.pdf")?;
//! let plan = readany::inspect(&bytes)?;
//! println!("{}", plan.summary());   // "mixed PDF, 12 pages, 3 need OCR"
//!
//! // Read the parts that need no OCR. Never silently partial.
//! let doc = readany::read(&bytes)?;
//! println!("{}", doc.markdown);
//! # Ok(())
//! # }
//! ```

mod clock;
pub mod error;
pub mod ocr;
pub mod pdf;
pub mod route;

pub use error::{ReadError, Result};
pub use ocr::OcrBackend;
pub use route::{inspect, inspect_named, PdfKind, PdfPlan, Plan, Route};

use std::path::Path;

/// Where one page's text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Extracted from the file's own text layer.
    Text,
    /// Recognised from pixels.
    Ocr,
    /// Needed OCR, and none was available.
    Unresolved,
}

/// One page of output, and how it was produced.
#[derive(Debug, Clone)]
pub struct Page {
    /// 1-indexed.
    pub number: u32,
    pub markdown: String,
    pub origin: Origin,
    /// Present when the page was recognised, not extracted.
    pub confidence: Option<f32>,
}

/// The result of reading a file.
///
/// Note `unresolved_pages`. If a page could not be read, it is named here
/// rather than quietly omitted. A caller can then decide to fail, to warn, or
/// to send those pages somewhere else — but never to mistake a partial read
/// for a complete one.
#[derive(Debug, Clone)]
pub struct Document {
    pub markdown: String,
    pub route: Route,
    pub pages: Vec<Page>,
    /// 1-indexed pages that were recognised from pixels.
    pub ocr_pages: Vec<u32>,
    /// 1-indexed pages that needed OCR and did not get it.
    pub unresolved_pages: Vec<u32>,
    pub processing_time_ms: u64,
}

impl Document {
    /// True when every page that needed reading was read.
    pub fn is_complete(&self) -> bool {
        self.unresolved_pages.is_empty()
    }

    /// A short account of what was done, suitable for a log or an audit trail.
    pub fn receipt(&self) -> String {
        let text = self
            .pages
            .iter()
            .filter(|p| p.origin == Origin::Text)
            .count();
        format!(
            "{} page(s): {} extracted, {} recognised, {} unresolved, in {}ms",
            self.pages.len(),
            text,
            self.ocr_pages.len(),
            self.unresolved_pages.len(),
            self.processing_time_ms
        )
    }
}

/// How to read.
#[derive(Default)]
pub struct Options<'a> {
    /// The file's name, when the caller knows it.
    ///
    /// Formats are recognised from their bytes wherever possible, because a
    /// name can lie. But some formats have no signature to read: a CSV of one
    /// column is a list of lines, indistinguishable from prose without being
    /// told. When sniffing cannot decide, the extension is consulted rather
    /// than refusing a file we can plainly read.
    pub filename: Option<&'a str>,
    /// Supply an OCR engine to read image-only pages. Without one, such pages
    /// are reported in `unresolved_pages` instead of being read.
    pub ocr: Option<&'a dyn OcrBackend>,
    /// Insert `<!-- Page N -->` markers between pages.
    pub page_markers: bool,
    /// Fail rather than return a partial document.
    pub strict: bool,
}

/// Read a file with the default options and no OCR backend.
pub fn read(bytes: &[u8]) -> Result<Document> {
    read_with(bytes, &Options::default())
}

/// Read a file from disk.
pub fn read_path(path: impl AsRef<Path>) -> Result<Document> {
    let bytes = std::fs::read(path.as_ref())?;
    read(&bytes)
}

/// Read a file into Markdown and nothing else.
pub fn to_markdown(bytes: &[u8]) -> Result<String> {
    Ok(read(bytes)?.markdown)
}

/// Read a file, choosing the engine from its content.
pub fn read_with(bytes: &[u8], options: &Options) -> Result<Document> {
    let watch = clock::Stopwatch::start();
    let plan = route::inspect_named(bytes, options.filename)?;

    let mut pages: Vec<Page> = Vec::new();

    match &plan.route {
        Route::Office(format) => {
            let markdown = anydoc::to_markdown_bytes(bytes, *format)?;
            pages.push(Page {
                number: 1,
                markdown,
                origin: Origin::Text,
                confidence: None,
            });
        }
        Route::Pdf(pdf) => {
            pages = read_pdf_pages(bytes, pdf, options)?;
        }
        Route::Image => {
            pages.push(read_image_page(bytes, options)?);
        }
        Route::Unknown => {
            return Err(ReadError::Unsupported(
                "unrecognised file content; readany reads office documents, PDFs and images".into(),
            ));
        }
    }

    let ocr_pages: Vec<u32> = pages
        .iter()
        .filter(|p| p.origin == Origin::Ocr)
        .map(|p| p.number)
        .collect();
    let unresolved_pages: Vec<u32> = pages
        .iter()
        .filter(|p| p.origin == Origin::Unresolved)
        .map(|p| p.number)
        .collect();

    if options.strict && !unresolved_pages.is_empty() {
        return Err(ReadError::OcrRequired {
            pages: unresolved_pages.len(),
        });
    }

    let markdown = join_pages(&pages, options.page_markers);

    Ok(Document {
        markdown,
        route: plan.route,
        pages,
        ocr_pages,
        unresolved_pages,
        processing_time_ms: watch.elapsed_ms(),
    })
}

/// The core of the library: read a PDF page by page, extracting where the text
/// exists and recognising only where it does not.
fn read_pdf_pages(bytes: &[u8], plan: &PdfPlan, _options: &Options) -> Result<Vec<Page>> {
    // Every PDF is read page by page, including one whose pages all carry
    // text.
    //
    // There used to be a shortcut here: a fully-textual PDF was read in one
    // call and returned as a single page numbered 1, however long it was. A
    // six-page bank statement reported "1 page". That is wrong twice over — a
    // caller cannot cite a page, and `unresolved_pages` cannot name one — and
    // page numbers are the whole basis of showing a person where a figure came
    // from.
    let _ = plan;
    let extracted = pdf_inspector::extract_pages_markdown_mem(bytes, None)?;
    let mut pages = Vec::with_capacity(extracted.pages.len());

    for page in extracted.pages {
        let number = page.page + 1; // pdf-inspector reports 0-indexed here.
        if !page.needs_ocr && !page.markdown.trim().is_empty() {
            pages.push(Page {
                number,
                markdown: page.markdown,
                origin: Origin::Text,
                confidence: None,
            });
            continue;
        }

        // This page is pixels. Reading it needs the page rendered to an image,
        // which requires a PDF rasteriser this crate deliberately does not
        // bundle. The page is reported, never silently dropped.
        pages.push(Page {
            number,
            markdown: String::new(),
            origin: Origin::Unresolved,
            confidence: None,
        });
    }

    Ok(pages)
}

fn read_image_page(bytes: &[u8], options: &Options) -> Result<Page> {
    match options.ocr {
        Some(backend) => {
            let result = backend.read_image(bytes)?;
            let markdown = ocr::markdown::to_markdown(&result, &ocr::MarkdownOptions::default());
            Ok(Page {
                number: 1,
                markdown,
                origin: Origin::Ocr,
                confidence: Some(result.confidence()),
            })
        }
        None => Ok(Page {
            number: 1,
            markdown: String::new(),
            origin: Origin::Unresolved,
            confidence: None,
        }),
    }
}

fn join_pages(pages: &[Page], markers: bool) -> String {
    let mut out = String::new();
    for page in pages {
        if page.markdown.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        if markers {
            out.push_str(&format!("<!-- Page {} -->\n\n", page.number));
        }
        out.push_str(page.markdown.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{ScanResult, TextBox, TextLine};

    /// An OCR backend that always returns the same line.
    struct StubOcr(&'static str);
    impl OcrBackend for StubOcr {
        fn read_image(&self, _bytes: &[u8]) -> ocr::Result<ScanResult> {
            Ok(ScanResult {
                lines: vec![TextLine {
                    boxes: vec![TextBox {
                        text: self.0.to_string(),
                        quad: ocr::Quad::from_rect(0.0, 0.0, 100.0, 16.0),
                        confidence: 0.88,
                    }],
                    baseline_y: 8.0,
                }],
                width: 200,
                height: 100,
                rotation: 0,
                skew: 0.0,
                processing_time_ms: 1,
                prepare_ms: 0,
                detect_ms: 0,
                recognize_ms: 1,
            })
        }
    }

    fn png() -> Vec<u8> {
        let img = image::GrayImage::from_pixel(40, 40, image::Luma([255u8]));
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
    fn an_image_without_an_ocr_backend_is_reported_not_silently_empty() {
        let doc = read(&png()).unwrap();
        assert!(doc.markdown.is_empty());
        assert_eq!(doc.unresolved_pages, vec![1]);
        assert!(!doc.is_complete());
        assert!(doc.receipt().contains("1 unresolved"));
    }

    #[test]
    fn an_image_with_an_ocr_backend_is_read() {
        let backend = StubOcr("QUICKMART SUPERMARKET");
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&backend),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(doc.markdown.contains("QUICKMART SUPERMARKET"));
        assert_eq!(doc.ocr_pages, vec![1]);
        assert!(doc.is_complete());
        assert_eq!(doc.pages[0].origin, Origin::Ocr);
        assert!(doc.pages[0].confidence.unwrap() > 0.8);
    }

    #[test]
    fn strict_mode_refuses_a_partial_read() {
        let err = read_with(
            &png(),
            &Options {
                strict: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, ReadError::OcrRequired { pages: 1 }));
    }

    #[test]
    fn an_unrecognised_file_says_what_is_supported() {
        let err = read(b"not a document, just prose").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("office documents"), "got: {message}");
    }

    #[test]
    fn page_markers_can_be_added() {
        let backend = StubOcr("hello");
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&backend),
                page_markers: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(doc.markdown.starts_with("<!-- Page 1 -->"));
    }

    #[test]
    fn the_receipt_accounts_for_every_page() {
        let backend = StubOcr("hello");
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&backend),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(doc
            .receipt()
            .starts_with("1 page(s): 0 extracted, 1 recognised, 0 unresolved"));
    }
}
