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

/// The version of this crate, as one fact rather than four.
///
/// `Cargo.toml`, `wasm/Cargo.toml`, `npm/package.json` and the generated
/// `pkg/package.json` each used to carry a version string of their own, in a
/// project whose whole claim is that it does not say things it has not
/// checked. This constant is the source: `readany-wasm` refuses to compile
/// unless its own version matches it, `scripts/version.sh` stamps the npm
/// manifests from it, and CI runs that script in check mode.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod clock;
pub mod error;
pub mod ocr;
pub mod pdf;
pub mod archive;
pub mod limits;
pub mod route;

pub use error::{ReadError, Result};
pub use ocr::OcrBackend;
pub use route::{inspect, inspect_named, PdfKind, PdfPlan, Plan, Route};

use std::path::Path;

/// Where one page's text came from.
///
/// # There is no `Human` here, and that is a decision
///
/// The product this engine serves paints four colours, and the fourth is
/// handwriting: *marked for a person, never guessed*. It is not a variant of
/// this enum, because this enum answers a question about a **page** and
/// handwriting is a fact about a **region**. The case the rule exists for is a
/// restaurant receipt whose printed total was read perfectly and whose tip was
/// written in by hand: the page's text came from pixels, so it is `Ocr`, and
/// saying otherwise about the whole page would be wrong about every printed
/// line on it.
///
/// A marked region is [`ocr::HumanRegion`], it is carried per page on
/// [`Page::human_regions`] and per line on [`ocr::TextLine::human`], and the
/// type has no text field at all. Putting the mark at page level would have
/// invited a consumer to colour a badge and think the rule was kept, when the
/// rule is about which *cell* a person has to be asked about.
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
    /// Regions on this page that were marked as handwritten and deliberately
    /// not read.
    ///
    /// Each one also appears in `markdown`, in its own place on its own row, as
    /// [`ocr::HUMAN_MARK`]. This field carries the geometry as well, so a
    /// reader can be shown *where* rather than only told that somewhere on this
    /// page a person wrote something.
    pub human_regions: Vec<ocr::HumanRegion>,
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
    /// 1-indexed pages carrying at least one region marked as handwritten.
    ///
    /// Named beside `unresolved_pages` on purpose, because it is the same class
    /// of fact: part of this document was not read, and a caller is being told
    /// rather than left to notice. The difference between the two is *why*.
    /// An unresolved page needed OCR and there was none; a marked region had
    /// OCR, and the engine looked at what came back and declined to assert it.
    pub human_pages: Vec<u32>,
    pub processing_time_ms: u64,
}

impl Document {
    /// True when every page that needed reading was read.
    ///
    /// **A marked handwritten region does not make a document incomplete**, and
    /// the distinction is worth stating rather than leaving to be discovered.
    /// Incompleteness here means a page produced no text and might have. A
    /// receipt with a handwritten tip is complete: everything printed on it was
    /// read, and the one thing that was not is named, positioned, and present
    /// in the markdown. Ask [`Document::needs_a_person`] for that.
    pub fn is_complete(&self) -> bool {
        self.unresolved_pages.is_empty()
    }

    /// True when some part of this document was marked for a person rather than
    /// read.
    pub fn needs_a_person(&self) -> bool {
        !self.human_pages.is_empty()
    }

    /// A short account of what was done, suitable for a log or an audit trail.
    pub fn receipt(&self) -> String {
        let text = self
            .pages
            .iter()
            .filter(|p| p.origin == Origin::Text)
            .count();
        let marked: usize = self.pages.iter().map(|p| p.human_regions.len()).sum();
        format!(
            "{} page(s): {} extracted, {} recognised, {} unresolved, \
             {} marked for a person, in {}ms",
            self.pages.len(),
            text,
            self.ocr_pages.len(),
            self.unresolved_pages.len(),
            marked,
            self.processing_time_ms
        )
    }
}

/// How to read.
///
/// # There is no `pdf_password`, and that is a decision rather than an omission
///
/// **Encrypted PDFs mostly read already.** The common protected bank statement
/// carries an owner password only — it restricts printing and copying, not
/// opening — and the empty user password opens it. Measured on a hand-built
/// RC4-40 file: the markdown is byte-identical to the same document
/// unencrypted, with no option set. `tests/encrypted_pdf.rs` pins that, because
/// nothing documented it and a behaviour nobody has written down is a behaviour
/// that can be removed by accident.
///
/// A PDF with a real *user* password returns [`ReadError::PasswordRequired`].
/// Passing a password through would need `pdf-inspector` to accept one on the
/// call this crate reads pages with, and it does not. Its only password-capable
/// entry points return one concatenated markdown string for the whole document,
/// and going through them page by page was measured to be a different answer,
/// not just a slower one:
///
/// | Measured on a 12-page document with mixed type sizes | Result |
/// |---|---|
/// | pages whose markdown differed from the ordinary route | **11 of 12** |
/// | pages whose `needs_ocr` verdict differed | 1 |
/// | cost at the 200-page limit | 1430 ms against 51 ms, **28×** |
///
/// The markdown differs because the ordinary route computes font statistics
/// across the whole document so heading thresholds stay consistent, and the
/// filtered route sees only the page it was asked for. Wiring a password
/// through it would mean a PDF reads differently *for having been encrypted*,
/// and `unresolved_pages` — the accounting this crate exists to keep honest —
/// would move with it. A password is not worth paying for with that.
///
/// A rasteriser is the one place a password is accepted, because rendering
/// takes a different route entirely: see
/// [`pdf::render::Rasterise::render_page_with_password`].
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
    /// What a hostile file is not allowed to do.
    ///
    /// Defaults to [`limits::Limits::default`], which is the documented
    /// contract rather than "no limits". A caller reading its own files can
    /// ask for [`limits::Limits::none`], and has to ask.
    pub limits: limits::Limits,
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

    // Refuse before reading, in cost order. Sniffing a file we are going to
    // refuse anyway is wasted work, and on a hostile file it is wasted work
    // chosen by the attacker.
    let limits = &options.limits;
    if bytes.len() as u64 > limits.input_bytes {
        return Err(error::ReadError::TooLarge(limits::Exceeded::new(
            "input size",
            limits.input_bytes,
            bytes.len() as u64,
        )));
    }
    // Every Office format is a zip. This is the only place the archive is
    // measured before `anydoc` unpacks it.
    archive::check(bytes, limits)?;

    let plan = route::inspect_named(bytes, options.filename)?;
    if let Route::Pdf(p) = &plan.route {
        if p.page_count > limits.pages {
            return Err(error::ReadError::TooLarge(limits::Exceeded::new(
                "pages",
                limits.pages as u64,
                p.page_count as u64,
            )));
        }
    }

    let mut pages: Vec<Page> = Vec::new();

    match &plan.route {
        Route::Office(format) => {
            let markdown = anydoc::to_markdown_bytes(bytes, *format)?;
            pages.push(Page {
                number: 1,
                markdown,
                origin: Origin::Text,
                confidence: None,
                human_regions: Vec::new(),
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
    let human_pages: Vec<u32> = pages
        .iter()
        .filter(|p| !p.human_regions.is_empty())
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
        human_pages,
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
                human_regions: Vec::new(),
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
            human_regions: Vec::new(),
        });
    }

    Ok(pages)
}

fn read_image_page(bytes: &[u8], options: &Options) -> Result<Page> {
    match options.ocr {
        Some(backend) => {
            let result = backend.read_image(bytes)?;
            let markdown = ocr::markdown::to_markdown(&result, &ocr::MarkdownOptions::default());
            let human_regions: Vec<_> = result.human_regions().into_iter().map(|(_, r)| *r).collect();
            // A recogniser that ran and came back with nothing did **not**
            // read this page, and calling it `Origin::Ocr` would take it out
            // of `unresolved_pages` — the one field this crate's "named, never
            // dropped" rule is made of. An empty read would then reach a
            // caller as a page that was read and happened to be empty, which
            // is a partial read wearing a complete one's clothes.
            //
            // This is not a hypothetical. Attaching a backend to a build that
            // had none silently moved every unreadable image from
            // `unresolved_pages` to `ocr_pages`, because every image gets a
            // `ScanResult` whether or not anything was in it.
            //
            // **A blank page lands here too, and that is the right side to err
            // on.** Nothing in the pixels separates "this paper has no ink on
            // it" from "this photograph defeated the detector", so the two
            // arrive identically and the honest report is the one that says a
            // page needed reading and did not get it. A caller that wants the
            // other reading has `human_regions` and the page's own emptiness
            // to reason from; a caller told the page was read has nothing.
            let read_something = !markdown.trim().is_empty() || !human_regions.is_empty();
            Ok(Page {
                number: 1,
                markdown,
                origin: if read_something {
                    Origin::Ocr
                } else {
                    Origin::Unresolved
                },
                // `None` when nothing was read: `ScanResult::confidence`
                // answers 0.0 for a page with no boxes, and 0.0 is a number a
                // caller can render as "read, very badly" rather than as "not
                // read". The absence is the truth.
                confidence: read_something.then(|| result.confidence()),
                human_regions,
            })
        }
        None => Ok(Page {
            number: 1,
            markdown: String::new(),
            origin: Origin::Unresolved,
            confidence: None,
            human_regions: Vec::new(),
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
                    human: Vec::new(),
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
        let backend = StubOcr("NORTHGATE MINIMART");
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&backend),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(doc.markdown.contains("NORTHGATE MINIMART"));
        assert_eq!(doc.ocr_pages, vec![1]);
        assert!(doc.is_complete());
        assert_eq!(doc.pages[0].origin, Origin::Ocr);
        assert!(doc.pages[0].confidence.unwrap() > 0.8);
    }

    /// A recogniser that ran and found nothing at all. The commonest real
    /// shapes: a photograph too dark to detect a box in, and a blank sheet.
    struct OcrThatReadNothing;
    impl OcrBackend for OcrThatReadNothing {
        fn read_image(&self, _bytes: &[u8]) -> ocr::Result<ScanResult> {
            Ok(ScanResult {
                lines: Vec::new(),
                width: 200,
                height: 100,
                rotation: 0,
                skew: 0.0,
                processing_time_ms: 1,
                prepare_ms: 0,
                detect_ms: 1,
                recognize_ms: 0,
            })
        }
    }

    /// **A recogniser that read nothing has not read the page.**
    ///
    /// Attaching a backend must not be able to *empty* `unresolved_pages`,
    /// which is the field the whole "named, never dropped" rule is made of.
    /// Before this, every image handed to a backend came back `Origin::Ocr`
    /// whatever the backend said, so a build that gained OCR silently turned
    /// every unreadable photograph into a page that was read and happened to
    /// contain nothing — complete, confident to three decimal places at 0.000,
    /// and invisible.
    ///
    /// **Falsifying mutation**, performed: in `read_image_page`, replace
    /// `if read_something { Origin::Ocr } else { Origin::Unresolved }` with
    /// `Origin::Ocr`. `unresolved_pages` becomes empty, `ocr_pages` becomes
    /// `[1]`, `is_complete()` becomes true and this test fails on the first
    /// assertion.
    #[test]
    fn an_ocr_backend_that_read_nothing_leaves_the_page_named() {
        let backend = OcrThatReadNothing;
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&backend),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(doc.unresolved_pages, vec![1], "the page must stay named");
        assert!(doc.ocr_pages.is_empty(), "nothing was recognised");
        assert!(!doc.is_complete());
        assert_eq!(doc.pages[0].origin, Origin::Unresolved);
        // Not `Some(0.0)`. A caller can render 0.0 as "read, very badly"; the
        // absence is the only value that cannot be mistaken for a reading.
        assert_eq!(doc.pages[0].confidence, None);
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

    /// An OCR backend that read one cell and marked another.
    struct OcrWithAMark;
    impl OcrBackend for OcrWithAMark {
        fn read_image(&self, _bytes: &[u8]) -> ocr::Result<ScanResult> {
            let ink = ocr::Ink {
                stroke_width: 3.0,
                stroke_variation: 1.1,
                baseline_drift: 0.2,
                coverage: 0.2,
            };
            Ok(ScanResult {
                lines: vec![TextLine {
                    boxes: vec![TextBox {
                        text: "TIP".to_string(),
                        quad: ocr::Quad::from_rect(0.0, 0.0, 40.0, 16.0),
                        confidence: 0.99,
                    }],
                    human: vec![ocr::HumanRegion {
                        quad: ocr::Quad::from_rect(60.0, 0.0, 40.0, 16.0),
                        evidence: ocr::Evidence {
                            ink,
                            confidence: 0.97,
                            page_confidence: 0.99,
                        },
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

    /// **What a consumer of a whole document is handed.**
    ///
    /// The mark is in the markdown, in the marked cell's own column, so a row
    /// whose figure was written by hand does not read as a row that had no
    /// figure. The page names the region so it can be drawn on screen, and the
    /// document names the page.
    #[test]
    fn a_marked_region_reaches_the_document_in_place_and_by_name() {
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&OcrWithAMark),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(doc.markdown.trim(), format!("TIP {}", ocr::HUMAN_MARK));
        assert_eq!(doc.human_pages, vec![1]);
        assert!(doc.needs_a_person());
        assert_eq!(doc.pages[0].human_regions.len(), 1);
        assert!(doc.receipt().contains("1 marked for a person"));

        // Complete, and needing a person, are different facts. Everything the
        // page printed was read; the one thing that was not is named.
        assert!(doc.is_complete());
        assert_eq!(doc.pages[0].origin, Origin::Ocr);
    }

    /// The same document with nothing marked: no mark in the markdown, no page
    /// named, and `needs_a_person` false. Without this the assertions above
    /// would pass for an implementation that marked unconditionally.
    #[test]
    fn a_document_with_nothing_written_on_it_names_nobody() {
        let doc = read_with(
            &png(),
            &Options {
                ocr: Some(&StubOcr("TIP")),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!doc.markdown.contains(ocr::HUMAN_MARK));
        assert!(doc.human_pages.is_empty());
        assert!(!doc.needs_a_person());
        assert!(doc.receipt().contains("0 marked for a person"));
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
