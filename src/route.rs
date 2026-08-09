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
    inspect_named(bytes, None)
}

/// Work out what a file is, given its name as well as its bytes.
pub fn inspect_named(bytes: &[u8], name: Option<&str>) -> Result<Plan> {
    let watch = crate::clock::Stopwatch::start();
    let route = classify_with_name(bytes, name)?;
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

fn classify_with_name(bytes: &[u8], name: Option<&str>) -> Result<Route> {
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

    // Delimited text has no signature to sniff. `anydoc::Format::from_bytes`
    // reads magic bytes, and a CSV has none, so a spreadsheet export — the
    // commonest file a bookkeeper will ever hand us — was being reported as
    // unrecognised while the documentation promised we read it.
    if let Some(format) = sniff_delimited(bytes) {
        return Ok(Route::Office(format));
    }

    // Last resort, and only for text we can actually decode. A name is weaker
    // evidence than a signature, so it is asked last and never allowed to
    // override what the bytes said.
    if let Some(format) = name
        .and_then(|n| n.rsplit('.').next())
        .and_then(anydoc::Format::from_extension)
    {
        if format != anydoc::Format::Pdf && std::str::from_utf8(bytes).is_ok() {
            return Ok(Route::Office(format));
        }
    }

    Ok(Route::Unknown)
}

/// Classify from bytes alone. Used by the tests, which is the point: the
/// bytes must be enough on their own for every format that has a signature.
#[cfg(test)]
fn classify(bytes: &[u8]) -> Result<Route> {
    classify_with_name(bytes, None)
}

/// Rows of a table, separated by something, quoted the way spreadsheets quote.
///
/// The test is consistency rather than any one character: real delimited text
/// has the same number of separators on nearly every line, and prose does not.
/// Counting outside quotes matters — a single cell of embedded JSON can hold
/// more commas than the whole rest of the row.
fn sniff_delimited(bytes: &[u8]) -> Option<anydoc::Format> {
    const LOOK_AT: usize = 12;
    const NEEDED: usize = 3;

    let text = std::str::from_utf8(bytes).ok()?;
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(LOOK_AT)
        .collect();
    if lines.len() < NEEDED {
        return None;
    }

    for delimiter in *b",;\t|" {
        let counts: Vec<usize> = lines.iter().map(|l| count_outside_quotes(l, delimiter)).collect();
        let first = counts[0];
        if first == 0 {
            continue;
        }
        let agreeing = counts.iter().filter(|&&c| c == first).count();
        // Every line but one must agree. A ragged file is prose that happens
        // to contain commas, and guessing at it would be worse than saying no.
        if agreeing + 1 >= counts.len() {
            return Some(anydoc::Format::Csv);
        }
    }
    None
}

fn count_outside_quotes(line: &str, delimiter: u8) -> usize {
    let mut inside = false;
    let mut count = 0;
    for b in line.bytes() {
        match b {
            b'"' => inside = !inside,
            d if d == delimiter && !inside => count += 1,
            _ => {}
        }
    }
    count
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

#[cfg(test)]
mod delimited_tests {
    use super::*;

    #[test]
    fn a_spreadsheet_export_is_recognised() {
        let csv = "Date,Amount,Balance\n2026-08-08,200.00,6332575.17\n2026-08-08,400.00,6332175.17\n2026-08-08,600.00,6331575.17\n";
        assert!(matches!(classify(csv.as_bytes()), Ok(Route::Office(_))));
    }

    /// A cell of embedded JSON holds more commas than the rest of the row.
    /// Counting them would make every line disagree and the file would be
    /// refused — which is exactly what happened to a real mobile-money export.
    #[test]
    fn commas_inside_a_quoted_cell_do_not_count() {
        let csv = "id,amount,meta\n1,200.00,\"{\"\"a\"\":1,\"\"b\"\":2,\"\"c\"\":3}\"\n2,400.00,\"{\"\"a\"\":4,\"\"b\"\":5,\"\"c\"\":6}\"\n3,600.00,\"{\"\"a\"\":7,\"\"b\"\":8,\"\"c\"\":9}\"\n";
        assert!(matches!(classify(csv.as_bytes()), Ok(Route::Office(_))));
    }

    #[test]
    fn semicolons_and_tabs_count_too() {
        let semi = "a;b;c\n1;2;3\n4;5;6\n";
        assert!(matches!(classify(semi.as_bytes()), Ok(Route::Office(_))));
        let tab = "a\tb\tc\n1\t2\t3\n4\t5\t6\n";
        assert!(matches!(classify(tab.as_bytes()), Ok(Route::Office(_))));
    }

    /// Prose with commas in it is not a table, and guessing would be worse
    /// than admitting we do not know what the file is.
    #[test]
    fn prose_is_not_mistaken_for_a_table() {
        let prose = "Dear Sir, I write regarding the matter.\nIt is, as you know, complicated.\nYours faithfully, Allan\nPost scriptum.\n";
        assert!(matches!(classify(prose.as_bytes()), Ok(Route::Unknown)));
    }

    #[test]
    fn two_lines_are_not_enough_to_be_sure() {
        assert!(matches!(classify(b"a,b,c\n1,2,3\n"), Ok(Route::Unknown)));
    }
}

#[cfg(test)]
mod name_hint_tests {
    use super::*;

    /// One column has no delimiters to be consistent about, so the bytes alone
    /// cannot separate it from prose. A list of account numbers is a perfectly
    /// ordinary thing for a bookkeeper to send.
    #[test]
    fn a_single_column_file_needs_its_name() {
        let one = b"reference\nINV-001\nINV-002\nINV-003\n";
        assert!(matches!(classify(one), Ok(Route::Unknown)));
        assert!(matches!(
            classify_with_name(one, Some("references.csv")),
            Ok(Route::Office(_))
        ));
    }

    /// A name is weaker evidence than a signature and must never overrule it.
    #[test]
    fn the_bytes_win_when_the_name_disagrees() {
        let csv = b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n";
        assert!(matches!(
            classify_with_name(csv, Some("actually_a_spreadsheet.docx")),
            Ok(Route::Office(anydoc::Format::Csv))
        ));
    }

    /// And a name alone is not enough for something we cannot even decode.
    #[test]
    fn a_name_cannot_rescue_bytes_we_cannot_read() {
        let junk = &[0xFFu8, 0xD8, 0x00, 0x01, 0x02, 0x03];
        assert!(matches!(
            classify_with_name(junk, Some("pretend.csv")),
            Ok(Route::Unknown)
        ));
    }
}
