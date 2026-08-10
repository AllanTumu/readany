//! WebAssembly bindings for readany.
//!
//! # What crosses this boundary, and what cannot
//!
//! Everything in the core crate that needs no neural runtime and no native
//! library runs here: routing, office documents, PDFs that carry a text layer,
//! the limits and their refusals, and the geometric preparation stages —
//! decode, crop, flatten, orient, deskew — which are pure Rust and work in a
//! browser today.
//!
//! **Recognition does not**, and neither does rasterising a PDF page.
//! [`readany::ocr::detect::Detector`], [`readany::ocr::recognize::Recognizer`]
//! and [`readany::pdf::render::Rasterise`] are traits with no implementation in
//! this crate or in this package; the implementations are native (ONNX, PDFium)
//! and live in `readany-ocr` and `readany-pdfium`, neither of which is
//! published. A page that needed recognition therefore comes back **named** in
//! `unresolved_pages` rather than silently dropped — which is the same promise
//! the Rust crate makes, kept the same way.
//!
//! One consequence worth stating because it is easy to misread as a bug: the
//! handwriting mark, `[handwritten]`, can never appear in this build's output.
//! Marking a region requires a recogniser to have run, and none can run here.
//! The field is present on every page so that a consumer written against this
//! package keeps working unchanged the day the work moves to a worker that has
//! one.
//!
//! # Errors
//!
//! Every failure is thrown as a real `Error` whose `name` is `ReadError` and
//! whose `kind` is one of the variants of [`readany::ReadError`], so a caller
//! can act on a refusal without matching on English prose. A refusal for
//! crossing a limit also carries `limit`, `allowed` and `found`.

use readany::limits::Limits;
use readany::ocr::ScanOptions;
use readany::{Options, Origin, PdfKind, ReadError, Route};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// The version of the `readany` crate this was built against.
///
/// Not a copy of it: the assertion below refuses to compile if the two ever
/// disagree, so there is one version in this project rather than one per
/// manifest.
#[wasm_bindgen(js_name = version)]
pub fn version() -> String {
    readany::VERSION.to_string()
}

const _: () = {
    const fn same(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }
    assert!(
        same(readany::VERSION, env!("CARGO_PKG_VERSION")),
        "wasm/Cargo.toml and Cargo.toml disagree about the version. \
         There is one version in this project; run scripts/version.sh."
    );
};

// ---------------------------------------------------------------------------
// What a caller may ask for
// ---------------------------------------------------------------------------

/// Options for [`read`], [`inspect`] and [`to_markdown`].
///
/// # Unknown keys are refused, and `deny_unknown_fields` is not what does it
///
/// A mistyped `pageMarkers` that silently did nothing would be exactly the
/// class of quiet failure this crate exists to refuse. The obvious spelling is
/// serde's `deny_unknown_fields`, and **it does not work through
/// `serde-wasm-bindgen`**: deserialising a struct from a JS object looks each
/// known field up by name rather than walking the object's keys, so an
/// unknown key is never visited and never rejected. It was written that way
/// here first, and a test passing `{ pageMarkers: true }` read the file
/// happily with page markers off.
///
/// So the keys are checked explicitly, against [`READ_OPTION_KEYS`]. The
/// attribute is gone rather than left in place looking like it is doing the
/// job.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ReadOptions {
    /// The file's name, when the caller knows it.
    ///
    /// Formats are read from their bytes wherever possible. This is consulted
    /// last and only for text that decodes, because a name can lie — but a
    /// one-column CSV has no signature to read, and without a name it is
    /// indistinguishable from prose.
    filename: Option<String>,
    /// Insert `<!-- Page N -->` markers between pages.
    page_markers: bool,
    /// Throw rather than return a document with unresolved pages.
    strict: bool,
    /// Numeric ceilings, applied on top of the defaults. Anything omitted keeps
    /// its default.
    limits: Option<LimitFields>,
    /// Run with no ceilings at all.
    ///
    /// Has to be asked for, and cannot be asked for at the same time as
    /// `limits`, so that "we forgot to configure limits" and "we chose to have
    /// none" cannot look the same in a review.
    no_limits: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LimitFields {
    input_bytes: Option<f64>,
    pages: Option<u32>,
    pixels_per_page: Option<f64>,
    decompressed_bytes: Option<f64>,
    archive_entries: Option<f64>,
    archive_depth: Option<u32>,
}

impl ReadOptions {
    fn limits(&self) -> Result<Limits, JsValue> {
        if self.no_limits {
            if self.limits.is_some() {
                return Err(plain(
                    "pass either `limits` or `no_limits`, not both: one raises \
                     individual ceilings, the other removes all of them",
                ));
            }
            return Ok(Limits::none());
        }
        let mut limits = Limits::default();
        let Some(f) = &self.limits else {
            return Ok(limits);
        };
        // A JS number is an f64, so a ceiling arrives as a double and has to be
        // checked rather than cast. `as u64` on a negative or a NaN is a
        // saturating conversion in Rust, which would quietly turn `-1` into a
        // limit of zero and `NaN` into a limit of zero as well — a caller
        // fat-fingering a ceiling would get a build that refuses every file and
        // no explanation.
        fn count(name: &str, v: Option<f64>) -> Result<Option<u64>, JsValue> {
            match v {
                None => Ok(None),
                Some(n) if n.is_finite() && n >= 0.0 && n <= (u64::MAX as f64) => {
                    Ok(Some(n as u64))
                }
                Some(n) => Err(plain(&format!(
                    "limits.{name} must be a non-negative finite number, got {n}"
                ))),
            }
        }
        if let Some(v) = count("input_bytes", f.input_bytes)? {
            limits.input_bytes = v;
        }
        if let Some(v) = f.pages {
            limits.pages = v;
        }
        if let Some(v) = count("pixels_per_page", f.pixels_per_page)? {
            limits.pixels_per_page = v;
        }
        if let Some(v) = count("decompressed_bytes", f.decompressed_bytes)? {
            limits.decompressed_bytes = v;
        }
        if let Some(v) = count("archive_entries", f.archive_entries)? {
            limits.archive_entries = v;
        }
        if let Some(v) = f.archive_depth {
            limits.archive_depth = v;
        }
        Ok(limits)
    }
}

const READ_OPTION_KEYS: &[&str] =
    &["filename", "page_markers", "strict", "limits", "no_limits"];
const LIMIT_KEYS: &[&str] = &[
    "input_bytes",
    "pages",
    "pixels_per_page",
    "decompressed_bytes",
    "archive_entries",
    "archive_depth",
];

/// Refuse a key we do not understand, naming it and what was expected.
///
/// See [`ReadOptions`] for why this is hand-written rather than
/// `deny_unknown_fields`.
fn reject_unknown_keys(value: &JsValue, allowed: &[&str], what: &str) -> Result<(), JsValue> {
    use wasm_bindgen::JsCast;
    let Some(object) = value.dyn_ref::<js_sys::Object>() else {
        // Not an object at all. serde will produce a better message about the
        // shape than anything invented here.
        return Ok(());
    };
    for key in js_sys::Object::keys(object).iter() {
        let Some(name) = key.as_string() else { continue };
        if !allowed.contains(&name.as_str()) {
            return Err(plain(&format!(
                "unknown {what} option {name:?}. Expected one of: {}",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

fn parse_options(options: JsValue) -> Result<ReadOptions, JsValue> {
    if options.is_undefined() || options.is_null() {
        return Ok(ReadOptions::default());
    }
    reject_unknown_keys(&options, READ_OPTION_KEYS, "read")?;
    // The nested object too. A ceiling misspelt inside `limits` is the same
    // defect one level down, and the more dangerous one: it would leave the
    // default ceiling in force while the caller believed it had raised it.
    if let Ok(limits) = js_sys::Reflect::get(&options, &JsValue::from_str("limits")) {
        if !limits.is_undefined() && !limits.is_null() {
            reject_unknown_keys(&limits, LIMIT_KEYS, "limits")?;
        }
    }
    serde_wasm_bindgen::from_value(options).map_err(|e| plain(&e.to_string()))
}

// ---------------------------------------------------------------------------
// What a caller gets back
// ---------------------------------------------------------------------------

/// What reading a file will involve, worked out without reading it.
#[derive(Serialize)]
struct PlanOut {
    /// "office", "pdf", "image" or "unknown".
    kind: String,
    /// The office format, when the route is "office".
    format: Option<String>,
    /// A sentence a person can read.
    summary: String,
    needs_ocr: bool,
    /// How many pages carry no text layer.
    ocr_page_count: usize,
    /// Present when the route is "pdf".
    pdf: Option<PdfOut>,
    inspect_time_ms: f64,
}

#[derive(Serialize)]
struct PdfOut {
    page_count: u32,
    /// 1-indexed pages with no usable text layer.
    pages_needing_ocr: Vec<u32>,
    /// "text", "mixed" or "scanned".
    kind: String,
    confidence: f32,
}

/// The result of reading a file.
#[derive(Serialize)]
struct DocumentOut {
    markdown: String,
    /// "office", "pdf", "image" or "unknown".
    route: String,
    page_count: usize,
    /// Every page, in order, with where its text came from.
    pages: Vec<PageOut>,
    /// Pages read from the file's own text layer.
    extracted: usize,
    /// 1-indexed pages recognised from pixels. Always empty in this build.
    ocr_pages: Vec<u32>,
    /// 1-indexed pages that needed recognition and did not get it.
    unresolved_pages: Vec<u32>,
    /// 1-indexed pages carrying a region marked as handwritten. Always empty in
    /// this build; marking one requires a recogniser to have run.
    human_pages: Vec<u32>,
    /// True when every page that needed reading was read.
    complete: bool,
    /// True when some part of the document was marked for a person rather than
    /// read. Distinct from `!complete`: a receipt whose printed lines were all
    /// read and whose tip was written by hand is complete *and* needs a person.
    needs_a_person: bool,
    /// A one-line account of what happened, for a log or an audit trail.
    receipt: String,
    processing_time_ms: f64,
}

#[derive(Serialize)]
struct PageOut {
    /// 1-indexed.
    number: u32,
    markdown: String,
    /// "text", "ocr" or "unresolved" — the provenance this product paints a
    /// colour for. Forest, amber and rose respectively.
    origin: String,
    /// Present when the page was recognised rather than extracted.
    confidence: Option<f32>,
    /// Regions the engine marked as handwritten and refused to read. There is
    /// no text on one, deliberately.
    human_regions: Vec<RegionOut>,
}

#[derive(Serialize)]
struct RegionOut {
    /// Clockwise from top-left, in image pixels.
    quad: [[f32; 2]; 4],
    /// What the ink looked like. The mark is made from these and not from the
    /// recogniser's confidence.
    ink: InkOut,
    /// The recogniser's confidence in the text that was discarded. Recorded,
    /// not consulted.
    confidence: f32,
    page_confidence: f32,
}

#[derive(Serialize)]
struct InkOut {
    stroke_width: f32,
    stroke_variation: f32,
    baseline_drift: f32,
    coverage: f32,
}

fn route_name(route: &Route) -> String {
    match route {
        Route::Office(_) => "office",
        Route::Pdf(_) => "pdf",
        Route::Image => "image",
        Route::Unknown => "unknown",
    }
    .to_string()
}

fn origin_name(origin: Origin) -> String {
    match origin {
        Origin::Text => "text",
        Origin::Ocr => "ocr",
        Origin::Unresolved => "unresolved",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// The calls
// ---------------------------------------------------------------------------

/// Work out what a file is and what reading it will cost. Reads nothing.
#[wasm_bindgen]
pub fn inspect(bytes: &[u8], options: JsValue) -> Result<JsValue, JsValue> {
    let opts = parse_options(options)?;
    let plan =
        readany::inspect_named(bytes, opts.filename.as_deref()).map_err(throw)?;

    let format = match &plan.route {
        Route::Office(f) => Some(format!("{f:?}")),
        _ => None,
    };
    let pdf = match &plan.route {
        Route::Pdf(p) => Some(PdfOut {
            page_count: p.page_count,
            pages_needing_ocr: p.pages_needing_ocr.clone(),
            kind: match p.kind {
                PdfKind::Text => "text",
                PdfKind::Mixed => "mixed",
                PdfKind::Scanned => "scanned",
            }
            .to_string(),
            confidence: p.confidence,
        }),
        _ => None,
    };

    let out = PlanOut {
        kind: route_name(&plan.route),
        format,
        summary: plan.summary(),
        needs_ocr: plan.needs_ocr,
        ocr_page_count: plan.ocr_page_count,
        pdf,
        inspect_time_ms: plan.inspect_time_ms as f64,
    };
    to_js(&out)
}

/// Read a file into Markdown, with a full account of every page.
#[wasm_bindgen]
pub fn read(bytes: &[u8], options: JsValue) -> Result<JsValue, JsValue> {
    let opts = parse_options(options)?;
    let doc = read_document(bytes, &opts)?;

    let extracted = doc
        .pages
        .iter()
        .filter(|p| p.origin == Origin::Text)
        .count();

    let out = DocumentOut {
        markdown: doc.markdown.clone(),
        route: route_name(&doc.route),
        page_count: doc.pages.len(),
        pages: doc
            .pages
            .iter()
            .map(|p| PageOut {
                number: p.number,
                markdown: p.markdown.clone(),
                origin: origin_name(p.origin),
                confidence: p.confidence,
                human_regions: p
                    .human_regions
                    .iter()
                    .map(|r| RegionOut {
                        quad: [
                            [r.quad.points[0].0, r.quad.points[0].1],
                            [r.quad.points[1].0, r.quad.points[1].1],
                            [r.quad.points[2].0, r.quad.points[2].1],
                            [r.quad.points[3].0, r.quad.points[3].1],
                        ],
                        ink: InkOut {
                            stroke_width: r.evidence.ink.stroke_width,
                            stroke_variation: r.evidence.ink.stroke_variation,
                            baseline_drift: r.evidence.ink.baseline_drift,
                            coverage: r.evidence.ink.coverage,
                        },
                        confidence: r.evidence.confidence,
                        page_confidence: r.evidence.page_confidence,
                    })
                    .collect(),
            })
            .collect(),
        extracted,
        ocr_pages: doc.ocr_pages.clone(),
        unresolved_pages: doc.unresolved_pages.clone(),
        human_pages: doc.human_pages.clone(),
        complete: doc.is_complete(),
        needs_a_person: doc.needs_a_person(),
        receipt: doc.receipt(),
        processing_time_ms: doc.processing_time_ms as f64,
    };
    to_js(&out)
}

/// Read a file and return only the Markdown.
#[wasm_bindgen(js_name = toMarkdown)]
pub fn to_markdown(bytes: &[u8], options: JsValue) -> Result<String, JsValue> {
    let opts = parse_options(options)?;
    Ok(read_document(bytes, &opts)?.markdown)
}

fn read_document(bytes: &[u8], opts: &ReadOptions) -> Result<readany::Document, JsValue> {
    let limits = opts.limits()?;
    readany::read_with(
        bytes,
        &Options {
            filename: opts.filename.as_deref(),
            // No backend can exist in this build. The field is left at its
            // default rather than hidden, because the reason it is empty is a
            // fact about the *package*, not about the document.
            ocr: None,
            page_markers: opts.page_markers,
            strict: opts.strict,
            limits,
        },
    )
    .map_err(throw)
}

/// The ceilings a job runs under when none are given.
///
/// The documented contract, not "no limits". Returned so a caller can show a
/// person why a file was refused without hard-coding numbers that may move.
#[wasm_bindgen(js_name = defaultLimits)]
pub fn default_limits() -> Result<JsValue, JsValue> {
    #[derive(Serialize)]
    struct Out {
        input_bytes: f64,
        pages: u32,
        pixels_per_page: f64,
        decompressed_bytes: f64,
        archive_entries: f64,
        archive_depth: u32,
        /// Enforced by killing the worker, never inside this module — and never
        /// at all in WebAssembly, which has no worker to kill. Reported so that
        /// one table states the whole contract.
        wall_clock_seconds: f64,
        resident_bytes: f64,
    }
    let l = Limits::default();
    to_js(&Out {
        input_bytes: l.input_bytes as f64,
        pages: l.pages,
        pixels_per_page: l.pixels_per_page as f64,
        decompressed_bytes: l.decompressed_bytes as f64,
        archive_entries: l.archive_entries as f64,
        archive_depth: l.archive_depth,
        wall_clock_seconds: l.wall_clock_seconds as f64,
        resident_bytes: l.resident_bytes as f64,
    })
}

// ---------------------------------------------------------------------------
// Preparation: the half of the OCR pipeline that needs no model
// ---------------------------------------------------------------------------

/// A page decoded and straightened, before any recognition.
///
/// This is the whole of the OCR pipeline that runs without a neural runtime,
/// and it is useful on its own: a browser can straighten a photograph locally
/// and send a corrected page to whatever does the recognising, rather than
/// sending the phone's original.
#[wasm_bindgen]
pub struct PreparedPage {
    width: u32,
    height: u32,
    rotation: u16,
    skew: f32,
    pixels: Vec<u8>,
}

#[wasm_bindgen]
impl PreparedPage {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Rotation applied before recognition, in degrees: 0, 90, 180 or 270.
    ///
    /// **Measured wrong five times out of five on real phone photographs**, and
    /// this build has no recogniser to settle it by reading. Treat it as a
    /// guess unless the page came from a scanner.
    #[wasm_bindgen(getter)]
    pub fn rotation(&self) -> u16 {
        self.rotation
    }
    /// Skew corrected before recognition, in degrees.
    #[wasm_bindgen(getter)]
    pub fn skew(&self) -> f32 {
        self.skew
    }
    /// The straightened page: 8-bit grayscale, row-major, `width * height`
    /// bytes.
    #[wasm_bindgen(getter)]
    pub fn pixels(&self) -> Vec<u8> {
        self.pixels.clone()
    }
}

/// Which set of corrections to apply. See `ScanOptions` in the Rust crate for
/// what each one was measured against.
#[derive(Default, Deserialize)]
#[serde(default)]
struct PrepareOptions {
    /// "photograph", "scanned_page" or "rendered_page". Defaults to
    /// "photograph".
    profile: Option<String>,
}

/// Decode a photograph or scan and straighten it. No model, no network.
///
/// The profile matters more than it sounds. `crop_to_content` finds the
/// document by texture, and on a bank statement the amount column is sparse
/// figures on white — texture detection reads that as background and cuts the
/// numbers off the page. Use "rendered_page" for anything that came out of a
/// PDF and "scanned_page" for anything that came off a scanner or a photocopier.
#[wasm_bindgen(js_name = prepare)]
pub fn prepare(bytes: &[u8], options: JsValue) -> Result<PreparedPage, JsValue> {
    let opts: PrepareOptions = if options.is_undefined() || options.is_null() {
        PrepareOptions::default()
    } else {
        reject_unknown_keys(&options, &["profile"], "prepare")?;
        serde_wasm_bindgen::from_value(options).map_err(|e| plain(&e.to_string()))?
    };
    let scan = match opts.profile.as_deref() {
        None | Some("photograph") => ScanOptions::for_photograph(),
        Some("scanned_page") => ScanOptions::for_scanned_page(),
        Some("rendered_page") => ScanOptions::for_rendered_page(),
        Some(other) => {
            return Err(plain(&format!(
                "unknown profile {other:?}: expected \"photograph\", \
                 \"scanned_page\" or \"rendered_page\""
            )))
        }
    };

    let prepared = readany::ocr::prepare_bytes(bytes, &scan)
        .map_err(|e| throw(ReadError::Ocr(e)))?;
    let (width, height) = prepared.image.dimensions();
    Ok(PreparedPage {
        width,
        height,
        rotation: prepared.rotation,
        skew: prepared.skew,
        pixels: prepared.image.into_raw(),
    })
}

// ---------------------------------------------------------------------------
// Errors that a caller can act on
// ---------------------------------------------------------------------------

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|e| plain(&e.to_string()))
}

/// An error in how this package was called, rather than in the document.
fn plain(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}

/// A refusal from the engine, thrown so a caller can match on `kind` instead of
/// on English.
///
/// The message is the crate's own `Display`, which for a limit names the limit
/// and the value that broke it. **No part of the document is in it.** The one
/// error that could carry a secret — a rejected PDF password — deliberately
/// does not carry the password.
fn throw(e: ReadError) -> JsValue {
    let error = js_sys::Error::new(&e.to_string());
    error.set_name("ReadError");

    let set = |key: &str, value: JsValue| {
        // Setting a property on a fresh Error cannot fail, and there is nothing
        // useful to do if it somehow did. Losing a field must not turn into
        // losing the error.
        let _ = js_sys::Reflect::set(&error, &JsValue::from_str(key), &value);
    };

    let kind = match &e {
        ReadError::Io(_) => "io",
        ReadError::Unsupported(_) => "unsupported",
        ReadError::Document(_) => "document",
        ReadError::Pdf(_) => "pdf",
        ReadError::PasswordRequired => "password_required",
        ReadError::PasswordRejected => "password_rejected",
        ReadError::TooLarge(_) => "too_large",
        ReadError::Ocr(_) => "ocr",
        ReadError::OcrRequired { .. } => "ocr_required",
    };
    set("kind", JsValue::from_str(kind));

    match &e {
        ReadError::TooLarge(exceeded) => {
            set("limit", JsValue::from_str(exceeded.limit));
            set("allowed", JsValue::from_f64(exceeded.allowed as f64));
            // `null` rather than absent when the true value is unknowable —
            // a stream still arriving when the cap was reached, or a product
            // that overflowed. "We did not measure it" and "it was zero" are
            // different facts.
            set(
                "found",
                match exceeded.found {
                    Some(found) => JsValue::from_f64(found as f64),
                    None => JsValue::NULL,
                },
            );
        }
        ReadError::OcrRequired { pages } => {
            set("pages", JsValue::from_f64(*pages as f64));
        }
        _ => {}
    }

    error.into()
}
