//! The OCR engine: straighten a page, find text, read it, order it.

use super::detect::Detector;
use super::error::Result;
use super::image::GrayImage;
use super::recognize::Recognizer;
use super::types::{Quad, ScanResult, TextBox};
use std::path::Path;

/// How to run the pipeline.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Even out the lighting before detection.
    ///
    /// A creased receipt has a fold shadow, and characters that fall in it are
    /// lost — not because they are unreadable but because the region growing
    /// never reaches them. Measured on a real receipt, this cost the first
    /// characters of four separate lines.
    pub flatten_lighting: bool,
    /// Find the document inside the photograph and read only that.
    ///
    /// A phone photograph of a receipt *can* be mostly table, and the detector
    /// shrinks whatever it is given to a fixed working size, so every glyph
    /// shrinks with the table.
    ///
    /// # How much table there actually was: measured, and it was not much
    ///
    /// That sentence used to end "cutting the document out first is worth more
    /// than any other single change measured on real photographs", which
    /// described a photograph nobody in the corpus had taken. Measured on the
    /// five, 4032 × 3024 each, reporting what
    /// [`crate::ocr::image::frame::content_bounds`] returned:
    ///
    /// | Page | kept | frame handed to the detector |
    /// |---|---|---|
    /// | 1 | 87.6% | 3951 × 2702 |
    /// | 2 | 61.3% | 4032 × 1855 |
    /// | 3 | — found nothing | 4032 × 3024 |
    /// | 4 | 78.7% | 4032 × 2379 |
    /// | 5 | — found nothing | 4032 × 3024 |
    ///
    /// **A person photographing a receipt fills the frame with it.** Two of the
    /// five were over `frame::MAX_AREA` and correctly declined, and the other
    /// three were trimmed rather than cut out. The whole shrink from frame to
    /// detector tensor is 1.58×, not the 4.2× the `max_side` 960 arithmetic
    /// predicts, because callers reading photographs already pass 2560.
    ///
    /// So cropping is worth having and is **not** what was wrong with the
    /// corpus. That was orientation — see [`Engine::scan_bytes`].
    pub crop_to_content: bool,
    /// Correct page rotation of 90, 180 or 270 degrees.
    pub fix_orientation: bool,
    /// Take the page as being turned this way, instead of measuring it.
    ///
    /// `None` is the normal setting and means "decide", by `fix_orientation`
    /// or, when that is off, by assuming the page is upright.
    ///
    /// This exists because measuring it does not work on a photograph.
    /// [`crate::ocr::image::orient::detect`] carries the table: on five real
    /// photographed receipts it was wrong five times, and wrong in the
    /// direction that calls a sideways page upright. Nothing in a projection
    /// profile separates them, because the biggest dark region in a photograph
    /// of a receipt is the shadow under it.
    ///
    /// So the engine stops guessing and reads the page each way up instead.
    /// [`Engine::scan_bytes`] sets this field on its later attempts, and keeps
    /// whichever attempt actually read — a measurement rather than a heuristic,
    /// taken from the recogniser that is already running.
    pub turn: Option<super::image::orient::Orientation>,
    /// Correct small skew introduced by scanners and phone cameras.
    pub fix_skew: bool,
    /// Below this mean confidence, [`ScanResult::needs_review`] returns true.
    pub confidence_floor: f32,
    /// When a page comes back below `confidence_floor`, read it again with the
    /// geometric corrections turned off and keep whichever attempt read best.
    ///
    /// The corrections are the usual cause of a bad read. Measured across seven
    /// photographed receipts, orientation was wrongly detected as 90 degrees on
    /// two of them; confidence collapsed to 0.198 and 0.219 and the text was
    /// nonsense. Reading those two again upright gave 0.960 and 0.925, and one
    /// went from 0 of 10 fields to 10 of 10.
    ///
    /// This costs a second pass only on the pages that failed, which on that
    /// set was two of seven.
    ///
    /// # This is the geometric retry, and it is the only one
    ///
    /// A *resolution* retry — read the page again rendered at 300 dpi — is a
    /// different thing, and it is deliberately not here. The engine could not
    /// perform one if it were: [`Engine::scan_bytes`] receives an encoded image
    /// and has never seen the PDF it came from, so only a caller holding both
    /// the document and a [`crate::pdf::render::Rasterise`] can render it
    /// again.
    ///
    /// It was measured before being declined, because a dpi retry sounds
    /// obviously good. **The gate this field uses never fires.** Across 32
    /// pages of five real PDFs and six renders of a generated statement, the
    /// page mean fell below `confidence_floor` exactly zero times.
    ///
    /// Worse, and the actual reason: on this pipeline mean confidence is not a
    /// correctness signal at all in that regime. The same statement rendered at
    /// 40 dpi gave back **not one** of its 74 known strings and reported a page
    /// mean of **0.647** — well above the 0.5 floor. The gate first says yes at
    /// 30 dpi, where the detector finds six boxes on a whole A4 page.
    ///
    /// So a dpi retry hung on this number would be dead code that reads as a
    /// safety net, and it would cost a second render and a second full
    /// inference pass to be it. 300 dpi genuinely does recover fine print —
    /// 58 of 74 fields to 74 of 74 at 4-point type — but confidence cannot tell
    /// you when to ask for it. `docs/ocr-dpi-sweep.md` has the whole table,
    /// including the control proving the gate can fire at all.
    pub retry_when_unsure: bool,
}

impl Default for ScanOptions {
    /// The photograph profile. See [`ScanOptions::for_photograph`].
    fn default() -> Self {
        ScanOptions::for_photograph()
    }
}

impl ScanOptions {
    /// A photograph: unknown lighting, tilt, orientation and background.
    ///
    /// Every correction on. These were tuned on seven real photographed
    /// receipts and each one earned its place there — cropping the document
    /// out of the table was worth more than any other single change.
    pub fn for_photograph() -> Self {
        ScanOptions {
            flatten_lighting: true,
            crop_to_content: true,
            fix_orientation: true,
            turn: None,
            fix_skew: true,
            confidence_floor: 0.5,
            retry_when_unsure: true,
        }
    }

    /// A page rendered from a PDF: clean, straight, upright, full bleed.
    ///
    /// Every geometric correction **off**, and this is not a small tuning
    /// preference. Measured on a rendered CaixaBank page at 300 dpi:
    ///
    /// | Profile | Handed to detector | Lines | Lines with digits |
    /// |---|---|---|---|
    /// | photograph | 560 × 2594 (22.6% of width) | 21 | **0** |
    /// | rendered page | 2480 × 3507 (100%) | **68** | **44** |
    ///
    /// `crop_to_content` is the one that does the damage. It finds the
    /// document by texture, and on a bank statement the merchant descriptions
    /// are dense while the amount and balance columns are sparse right-aligned
    /// figures on white. Texture detection reads that white space as
    /// background and cuts the page down to the dense column, so **the numbers
    /// never reach the network at all**.
    ///
    /// That single fact explained a whole set of flat measurements: dpi from
    /// 150 to 400, `max_side` from 960 to 3200 and both detector thresholds
    /// all changed nothing, because every one of them operated on an image the
    /// amounts had already been cut out of.
    ///
    /// The other three corrections are harmless here but pointless: a rendered
    /// page has no fold shadow to flatten, no skew to estimate and no
    /// orientation to guess. Leaving them on costs about 2.7× the time for an
    /// identical result.
    pub fn for_rendered_page() -> Self {
        ScanOptions {
            flatten_lighting: false,
            crop_to_content: false,
            fix_orientation: false,
            turn: None,
            fix_skew: false,
            confidence_floor: 0.5,
            // Nothing to retry *with*: the corrections that a retry turns off
            // are already off. A rendered page could in principle be retried at
            // a higher dpi instead — that was measured and declined, because
            // the gate fires on no page this engine has ever been shown. See
            // `retry_when_unsure`.
            retry_when_unsure: false,
        }
    }

    /// A scanned or photocopied page: straight-ish, full bleed, tilted.
    ///
    /// **A scan is neither of the two profiles above, and reading it as either
    /// loses the document.** It is not a photograph — it has no background to
    /// crop away, and `crop_to_content` is measured above as the one correction
    /// that cuts a bank statement down to its densest block and takes the
    /// amount column with it. But neither is it a clean render: it arrives off
    /// square, because paper goes through a feeder crooked and a phone is never
    /// held level.
    ///
    /// So: every correction on **except** cropping.
    ///
    /// Measured on a generated eighteen-row statement rendered at 150 dpi and
    /// then tilted, read through the full pipeline to a verdict:
    ///
    /// | Tilt | `for_rendered_page` | this profile |
    /// |---|---|---|
    /// | 1° | 7 of 18 rows, `Failed` | **18 of 18, verified** |
    /// | 4° | 0 of 18, `Failed` | **18 of 18, verified** |
    /// | 12° | 0 of 18, `NotThisKind` | **18 of 18, verified** |
    /// | 270° | 0 of 18, `NotThisKind` | **18 of 18, verified** |
    /// | 90° | 0 of 18, `NotThisKind` | **18 of 18, verified** |
    ///
    /// The 90° row was `0 of 18` under this profile too until the retry ladder
    /// grew its quarter turns — orientation detection recovered 270° and not
    /// 90°, and neither the profile nor the detector changed to fix it. See
    /// [`Engine::scan_bytes`]. 180° is still refused under every profile, and
    /// correctly: an upside-down page and an upright one have the same
    /// projection, so only a classifier could separate them and none is here.
    ///
    /// **One degree of tilt was the difference between reading a statement and
    /// refusing it**, and one degree is not a damaged document — it is a sheet
    /// fed slightly crooked. Nothing was wrong with the recognition; the page
    /// was simply never straightened, because the only profile that straightens
    /// pages also crops them.
    ///
    /// `retry_when_unsure` is on here and off for a rendered page, and the
    /// difference is real rather than cosmetic: there are now corrections for a
    /// retry to turn off.
    pub fn for_scanned_page() -> Self {
        ScanOptions {
            // A photocopy has a fold shadow and a lid gap; a feeder scan has a
            // bright edge. All of it is uneven lighting.
            flatten_lighting: true,
            // The one that does the damage. See the table above.
            crop_to_content: false,
            fix_orientation: true,
            turn: None,
            fix_skew: true,
            confidence_floor: 0.5,
            retry_when_unsure: true,
        }
    }
}

/// A page after geometric correction, before recognition.
#[derive(Debug)]
pub struct Prepared {
    /// The straightened page. Detection runs on this.
    pub image: GrayImage,
    /// The original pixels, untouched. Crops are sampled from here so a glyph
    /// is only ever resampled once.
    pub original: GrayImage,
    /// How to get from a point on `image` back to a point on `original`.
    pub correction: super::image::Correction,
    pub rotation: u16,
    pub skew: f32,
}

/// Decode a page and straighten it. This needs no model and no network,
/// so it works today and is useful on its own.
pub fn prepare_bytes(bytes: &[u8], options: &ScanOptions) -> Result<Prepared> {
    prepare_bytes_using(bytes, options, None)
}

/// As [`prepare_bytes`], with a platform decoder for containers this crate
/// cannot open — HEIC above all. See [`super::image::DecodeImage`].
pub fn prepare_bytes_using(
    bytes: &[u8],
    options: &ScanOptions,
    decoder: Option<&dyn super::image::DecodeImage>,
) -> Result<Prepared> {
    let whole = super::image::decode_bytes_using(bytes, &crate::limits::Limits::default(), decoder)?;

    // Cut the document out of the photograph before anything else. Everything
    // downstream then works on paper rather than on a table.
    let (decoded, crop_origin) = match options
        .crop_to_content
        .then(|| super::image::frame::content_bounds(&whole))
        .flatten()
    {
        Some((x, y, w, h)) => (
            image::imageops::crop_imm(&whole, x, y, w, h).to_image(),
            (x, y),
        ),
        None => (whole, (0, 0)),
    };

    // Flattening is for the detector's benefit only. The recogniser still gets
    // its crops from the untouched original, because every extra resample of a
    // glyph costs accuracy.
    let evened = if options.flatten_lighting {
        super::image::flatten::flatten(&decoded)
    } else {
        decoded.clone()
    };

    let orientation = match options.turn {
        // An answer supplied by the caller is not second-guessed. This is how
        // `scan_bytes` reads a page the other way up.
        Some(turn) => turn,
        None if options.fix_orientation => super::image::orient::detect(&evened),
        None => super::image::orient::Orientation::Upright,
    };
    let turned = super::image::orient::apply(&evened, orientation);
    let rotation = orientation.degrees();

    let turned_dims = turned.dimensions();
    let (straight, skew) = if options.fix_skew {
        let skew = super::image::deskew::estimate_skew(&turned);
        (super::image::deskew::rotate(&turned, -skew), skew)
    } else {
        (turned, 0.0)
    };

    let correction = super::image::Correction {
        orientation,
        skew,
        original: decoded.dimensions(),
        turned: turned_dims,
        crop_origin,
    };

    Ok(Prepared {
        image: straight,
        original: decoded,
        correction,
        rotation,
        skew,
    })
}

/// Decode and straighten a page from disk.
pub fn prepare(path: impl AsRef<Path>) -> Result<Prepared> {
    let bytes = std::fs::read(path.as_ref())?;
    prepare_bytes(&bytes, &ScanOptions::default())
}

/// The full pipeline, once a detector and a recogniser are supplied.
///
/// The backends are generic on purpose. The core crate stays free of a neural
/// runtime, so it builds for WebAssembly and for machines with no GPU, and the
/// same pipeline can be driven by ONNX, by a remote service, or by a stub in
/// tests.
pub struct Engine<D: Detector, R: Recognizer> {
    detector: D,
    recognizer: R,
    options: ScanOptions,
    decoder: Option<Box<dyn super::image::DecodeImage>>,
}

impl<D: Detector, R: Recognizer> Engine<D, R> {
    pub fn new(detector: D, recognizer: R) -> Self {
        Engine {
            detector,
            recognizer,
            options: ScanOptions::default(),
            decoder: None,
        }
    }

    pub fn with_options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Supply the platform's decoder for containers this crate cannot open.
    ///
    /// Without one, a HEIC photograph — which is what an iPhone writes unless
    /// it is told otherwise — comes back as
    /// [`crate::ocr::ScanError::NeedsPlatformDecoder`] and is never read. See
    /// [`super::image::DecodeImage`].
    pub fn with_decoder(mut self, decoder: Box<dyn super::image::DecodeImage>) -> Self {
        self.decoder = Some(decoder);
        self
    }

    /// Read a page: straighten, detect, recognise, order.
    ///
    /// If the first attempt comes back unsure and `retry_when_unsure` is set,
    /// the page is read again a different way up and the better attempt is
    /// returned. A wrong rotation is the commonest way a perfectly readable
    /// page turns into nonsense, and the engine can tell that it happened, so
    /// it should not hand the nonsense back.
    ///
    /// # The ladder, and why the last two rungs exist
    ///
    /// The first two retries turn the geometric corrections **off**, for the
    /// page whose corrections were the problem. The last two turn the page a
    /// quarter, for the page whose correction never fired at all — and that is
    /// the whole of the receipt corpus. Measured on five real photographed
    /// receipts, all five genuinely on their side and all five called upright
    /// by [`crate::ocr::image::orient::detect`]:
    ///
    /// | | as read before | with the quarter turns |
    /// |---|---|---|
    /// | boxes a page | 12–22 | 18–46 |
    /// | characters a box | **1.0–1.1** | **8.7–15.0** |
    /// | mean confidence | 0.22–0.29 | 0.80–0.97 |
    /// | below the floor | 5 of 5 | 0 of 5 |
    ///
    /// One character a box is the signature of a page handed to the recogniser
    /// sideways: a line of type becomes a tall narrow crop, and a tall narrow
    /// crop scaled to the recogniser's fixed height is a few pixels wide.
    ///
    /// **This is a measurement standing in for a heuristic.** No projection
    /// statistic separates those two columns — several were tried and are
    /// recorded on `orient::detect` — but the recogniser's own output separates
    /// them by a factor of ten, and the recogniser is already running.
    ///
    /// # Cost
    ///
    /// Nothing at all for a page that reads first time, which is the common
    /// case: the ladder is entered only below `confidence_floor` and stops at
    /// the first rung that clears it. A rung whose preparation would be
    /// identical to one already tried is skipped rather than run, so a page
    /// that was found upright and unskewed does not pay for two retries that
    /// would decode the same pixels again.
    pub fn scan_bytes(&self, bytes: &[u8]) -> Result<ScanResult> {
        let mut best = self.scan_once(bytes, &self.options)?;
        if !self.options.retry_when_unsure || best.confidence() >= self.options.confidence_floor {
            return Ok(best);
        }

        use super::image::orient::Orientation;
        let corrections_fired = best.rotation != 0 || best.skew.abs() >= 0.01;
        let attempts = [
            // Rungs 1 and 2: the corrections were the problem. Worth running
            // only if they actually did something on the first attempt.
            corrections_fired.then(|| ScanOptions {
                fix_orientation: false,
                turn: None,
                ..self.options.clone()
            }),
            corrections_fired.then(|| ScanOptions {
                fix_orientation: false,
                turn: None,
                fix_skew: false,
                ..self.options.clone()
            }),
            // Rungs 3 and 4: the page is on its side and nothing said so. Only
            // offered to a profile that admits it does not know which way up
            // the page is — a rendered page does, and does not retry at all.
            self.options.fix_orientation.then(|| ScanOptions {
                turn: Some(Orientation::Rotated90),
                ..self.options.clone()
            }),
            self.options.fix_orientation.then(|| ScanOptions {
                turn: Some(Orientation::Rotated270),
                ..self.options.clone()
            }),
        ];

        // Every way up already read, so a rung that would repeat one is skipped
        // rather than paying for a second identical inference pass.
        let mut read_at: Vec<u16> = vec![best.rotation];
        for attempt in attempts.into_iter().flatten() {
            if let Some(turn) = attempt.turn {
                if read_at.contains(&turn.degrees()) {
                    continue;
                }
            }
            let again = self.scan_once(bytes, &attempt)?;
            read_at.push(again.rotation);
            if read_mass(&again) > read_mass(&best) {
                best = again;
            }
            if best.confidence() >= self.options.confidence_floor {
                break;
            }
        }
        Ok(best)
    }

    fn scan_once(&self, bytes: &[u8], options: &ScanOptions) -> Result<ScanResult> {
        let watch = crate::clock::Stopwatch::start();
        let stage = crate::clock::Stopwatch::start();
        let prepared = prepare_bytes_using(bytes, options, self.decoder.as_deref())?;
        let (width, height) = prepared.image.dimensions();
        let prepare_ms = stage.elapsed_ms();

        let stage = crate::clock::Stopwatch::start();
        let quads = self.detector.detect(&prepared.image)?;
        let detect_ms = stage.elapsed_ms();

        // Crop everything first, then hand the whole set to the recogniser in
        // one call. A backend that batches pays the cost of entering the
        // inference runtime once instead of once per box, which on a phone is
        // most of the time spent. Backends that do not batch get the same
        // answer from the looping default on the trait.
        let crops: Vec<GrayImage> = quads
            .iter()
            .map(|quad| crop_corrected(&prepared, quad))
            .collect();
        let stage = crate::clock::Stopwatch::start();
        let read = self.recognizer.recognize_batch(&crops)?;
        let recognize_ms = stage.elapsed_ms();

        let mut boxes = Vec::with_capacity(quads.len());
        for (quad, (text, confidence)) in quads.into_iter().zip(read) {
            if text.trim().is_empty() {
                continue;
            }
            boxes.push(TextBox {
                text,
                quad,
                confidence,
            });
        }

        let lines = super::layout::assemble(boxes, width as f32);

        Ok(ScanResult {
            lines,
            width,
            height,
            rotation: prepared.rotation,
            skew: prepared.skew,
            processing_time_ms: watch.elapsed_ms(),
            prepare_ms,
            detect_ms,
            recognize_ms,
        })
    }

    pub fn scan(&self, path: impl AsRef<Path>) -> Result<ScanResult> {
        let bytes = std::fs::read(path.as_ref())?;
        self.scan_bytes(&bytes)
    }

    /// Read a page and return Markdown.
    pub fn to_markdown(&self, path: impl AsRef<Path>) -> Result<String> {
        let result = self.scan(path)?;
        Ok(super::markdown::to_markdown(
            &result,
            &super::markdown::MarkdownOptions::default(),
        ))
    }
}

/// How much of the page an attempt actually read, for choosing between
/// attempts. Characters, each weighted by how sure the recogniser was of it.
///
/// **Not mean confidence, and the difference is not cosmetic.** Mean confidence
/// is an average over whatever the detector happened to find, so an attempt
/// that found three boxes and read them well scores above one that found forty
/// and read them well. With two rungs on the ladder that was survivable; with
/// four it is a real way to pick the wrong page, and this crate has already
/// written down that mean confidence answers "was what I read hard to read",
/// never "did I read everything" — see [`TextBox::confidence`].
///
/// This asks the second question. It is still not a correctness signal and is
/// not used as one: it decides only which of two readings of the *same page* to
/// keep, where more text read more surely is the better reading by definition.
fn read_mass(result: &ScanResult) -> f32 {
    result
        .lines
        .iter()
        .flat_map(|l| l.boxes.iter())
        .map(|b| b.confidence * b.text.chars().count() as f32)
        .sum()
}

/// Cut a detected box out of the **original** pixels.
///
/// The box was found on the straightened page, so each output pixel is mapped
/// back through the correction and sampled from the original. That is one
/// resample instead of two, and it is worth several percent of accuracy on a
/// tilted page.
fn crop_corrected(prepared: &Prepared, quad: &Quad) -> GrayImage {
    if prepared.correction.is_identity() {
        return crop_quad(&prepared.image, quad);
    }

    let (x, y, w, h) = quad.bbox();
    let cw = w.ceil().max(1.0) as u32;
    let ch = h.ceil().max(1.0) as u32;
    let (ow, oh) = prepared.original.dimensions();

    let mut out = GrayImage::from_pixel(cw, ch, image::Luma([255u8]));
    for v in 0..ch {
        for u in 0..cw {
            let (sx, sy) = prepared.correction.to_original(x + u as f32, y + v as f32);
            if sx < 0.0 || sy < 0.0 || sx >= (ow - 1) as f32 || sy >= (oh - 1) as f32 {
                continue;
            }
            out.put_pixel(u, v, image::Luma([sample(&prepared.original, sx, sy)]));
        }
    }
    out
}

/// Bilinear sample, so a fractional coordinate does not snap to a pixel.
fn sample(img: &GrayImage, x: f32, y: f32) -> u8 {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let p = |px: u32, py: u32| img.get_pixel(px, py).0[0] as f32;
    let top = p(x0, y0) * (1.0 - fx) + p(x0 + 1, y0) * fx;
    let bottom = p(x0, y0 + 1) * (1.0 - fx) + p(x0 + 1, y0 + 1) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

/// Cut the axis-aligned bounding box of a quad out of the page.
fn crop_quad(img: &GrayImage, quad: &Quad) -> GrayImage {
    let (x, y, w, h) = quad.bbox();
    let (iw, ih) = img.dimensions();
    let x0 = x.max(0.0) as u32;
    let y0 = y.max(0.0) as u32;
    let x1 = ((x + w).ceil() as u32).min(iw);
    let y1 = ((y + h).ceil() as u32).min(ih);
    if x1 <= x0 || y1 <= y0 {
        return GrayImage::new(1, 1);
    }
    let mut out = GrayImage::new(x1 - x0, y1 - y0);
    for (dy, row) in (y0..y1).enumerate() {
        for (dx, col) in (x0..x1).enumerate() {
            out.put_pixel(dx as u32, dy as u32, *img.get_pixel(col, row));
        }
    }
    out
}

#[cfg(test)]
mod profile_tests {
    use super::ScanOptions;

    #[test]
    fn the_rendered_page_profile_does_not_crop() {
        // The whole bug: `crop_to_content` cut a bank statement down to its
        // description column, so the amounts never reached the network.
        // Measured 2480x3507 -> 560x2594, and 44 lines with digits -> 0.
        let o = ScanOptions::for_rendered_page();
        assert!(!o.crop_to_content, "cropping a rendered page removes its columns");
        assert!(!o.flatten_lighting, "a rendered page has no shadow to flatten");
        assert!(!o.fix_skew, "a rendered page is straight");
        assert!(!o.fix_orientation, "a rendered page is upright");
    }

    #[test]
    fn the_photograph_profile_keeps_every_correction() {
        // Each was earned on seven real photographed receipts.
        let o = ScanOptions::for_photograph();
        assert!(o.crop_to_content);
        assert!(o.flatten_lighting);
        assert!(o.fix_skew);
        assert!(o.fix_orientation);
        assert!(o.retry_when_unsure);
    }

    #[test]
    fn the_default_is_the_photograph_profile() {
        // Unchanged behaviour for every existing caller: only the rasteriser
        // path opts into the new profile.
        let d = ScanOptions::default();
        let p = ScanOptions::for_photograph();
        assert_eq!(d.crop_to_content, p.crop_to_content);
        assert_eq!(d.flatten_lighting, p.flatten_lighting);
        assert_eq!(d.fix_skew, p.fix_skew);
        assert_eq!(d.fix_orientation, p.fix_orientation);
    }

    #[test]
    fn the_two_profiles_are_not_the_same() {
        assert_ne!(
            ScanOptions::for_photograph().crop_to_content,
            ScanOptions::for_rendered_page().crop_to_content,
            "a page render and a photograph must not share preparation defaults"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::ScanOptions;

    /// The three profiles differ in the ways that earned them.
    ///
    /// Each pair is separated by a *named* field rather than by inequality of
    /// the whole struct, because "these two are different somehow" is not the
    /// claim — the claim is that a scan is straightened where a render is not,
    /// and left uncropped where a photograph is not. An equality over the whole
    /// struct would keep passing if the distinguishing field were the one that
    /// changed back.
    #[test]
    fn a_scan_is_straightened_like_a_photograph_and_uncropped_like_a_render() {
        let (photo, render, scan) = (
            ScanOptions::for_photograph(),
            ScanOptions::for_rendered_page(),
            ScanOptions::for_scanned_page(),
        );

        // Against a render: a scan arrives off square, and must be corrected.
        // One degree of tilt was the difference between reading an eighteen-row
        // statement and refusing it.
        assert!(!render.fix_skew && !render.fix_orientation);
        assert!(scan.fix_skew && scan.fix_orientation);

        // Against a photograph: cropping finds the document by texture, and on
        // a statement that means the densest block — which takes the amount
        // column off the page.
        assert!(photo.crop_to_content);
        assert!(!scan.crop_to_content);

        // And the retry has something to turn off here, which is why a render
        // does not offer one.
        assert!(scan.retry_when_unsure);
        assert!(!render.retry_when_unsure);
    }

    /// These tests are about orientation and skew, so they hand the pipeline a
    /// small synthetic page and expect its coordinates back unchanged. Finding
    /// a document inside a photograph is a different job with its own tests in
    /// `image::frame`, and it would cut these pages before they were measured.
    fn geometry_only() -> ScanOptions {
        ScanOptions {
            crop_to_content: false,
            ..Default::default()
        }
    }
    use super::*;
    use crate::ocr::detect::Detector;
    use crate::ocr::error::ScanError;
    use crate::ocr::recognize::Recognizer;

    /// A detector that returns fixed boxes, so the pipeline can be tested
    /// without a model.
    struct FakeDetector(Vec<Quad>);
    impl Detector for FakeDetector {
        fn detect(&self, _image: &GrayImage) -> Result<Vec<Quad>> {
            Ok(self.0.clone())
        }
    }

    /// A recogniser that reports the crop size, so we can prove the pipeline
    /// cropped the right region.
    struct FakeRecognizer;
    impl Recognizer for FakeRecognizer {
        fn recognize(&self, crop: &GrayImage) -> Result<(String, f32)> {
            Ok((format!("{}x{}", crop.width(), crop.height()), 0.9))
        }
    }

    fn png_page() -> Vec<u8> {
        let mut img = GrayImage::from_pixel(200, 120, ::image::Luma([255u8]));
        for y in 20..30 {
            for x in 10..190 {
                img.put_pixel(x, y, ::image::Luma([0u8]));
            }
        }
        let mut bytes = Vec::new();
        ::image::DynamicImage::ImageLuma8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                ::image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn prepare_decodes_and_reports_corrections() {
        let prepared = prepare_bytes(&png_page(), &geometry_only()).unwrap();
        assert_eq!(prepared.image.dimensions(), (200, 120));
        assert!(prepared.skew.abs() < 2.0);
    }

    #[test]
    fn a_non_image_is_rejected_clearly() {
        let err =
            prepare_bytes(b"this is not an image at all", &ScanOptions::default()).unwrap_err();
        assert!(
            matches!(err, ScanError::Unsupported(_) | ScanError::Decode(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn the_pipeline_runs_end_to_end_with_stub_backends() {
        let engine = Engine::new(
            FakeDetector(vec![
                Quad::from_rect(10.0, 60.0, 80.0, 20.0),
                Quad::from_rect(10.0, 20.0, 180.0, 10.0),
            ]),
            FakeRecognizer,
        )
        .with_options(geometry_only());
        let result = engine.scan_bytes(&png_page()).unwrap();
        // Two boxes on different rows become two lines, top one first.
        assert_eq!(result.lines.len(), 2);
        assert_eq!(result.lines[0].text(), "180x10");
        assert_eq!(result.lines[1].text(), "80x20");
        assert!(result.confidence() > 0.8);
        assert!(!result.needs_review(0.5));
    }

    #[test]
    fn crops_come_from_the_original_when_the_page_was_straightened() {
        // A tilted page: the correction is not identity, so the crop path
        // must sample the original rather than the resampled copy.
        let prepared = prepare_bytes(&png_page(), &geometry_only()).unwrap();
        assert_eq!(prepared.original.dimensions(), (200, 120));
        // Whatever the measured skew, mapping the centre back must land
        // inside the original image.
        let (cx, cy) = prepared.correction.to_original(100.0, 60.0);
        assert!(
            (0.0..200.0).contains(&cx) && (0.0..120.0).contains(&cy),
            "{cx},{cy}"
        );
    }

    #[test]
    fn a_page_with_no_detections_is_flagged_for_review() {
        let engine = Engine::new(FakeDetector(Vec::new()), FakeRecognizer);
        let result = engine.scan_bytes(&png_page()).unwrap();
        assert!(result.lines.is_empty());
        assert!(result.needs_review(0.5));
    }

    /// A caller's answer beats the measurement, because the measurement is
    /// what was wrong on every real photograph.
    #[test]
    fn a_supplied_turn_is_applied_instead_of_being_detected() {
        let sideways = ScanOptions {
            turn: Some(crate::ocr::image::orient::Orientation::Rotated90),
            ..geometry_only()
        };
        let prepared = prepare_bytes(&png_page(), &sideways).unwrap();
        assert_eq!(prepared.rotation, 90, "the supplied turn was not applied");
        // A quarter turn swaps the page's sides. 200x120 in, 120x200 out.
        assert_eq!(prepared.image.dimensions(), (120, 200));
        assert_eq!(
            prepared.correction.orientation,
            crate::ocr::image::orient::Orientation::Rotated90,
            "the crop path must know which way the page was turned, or every \
             box maps back to the wrong pixels"
        );
    }

    /// A recogniser that reads well only when the crop is wider than it is
    /// tall, which is what a line of type looks like the right way up. This is
    /// the corpus's defect in miniature: sideways, every crop is a tall sliver
    /// and comes back as one unsure character.
    struct OnlyReadsUpright;
    impl Recognizer for OnlyReadsUpright {
        fn recognize(&self, crop: &GrayImage) -> Result<(String, f32)> {
            if crop.width() > crop.height() {
                Ok(("ELEVEN CHAR".to_string(), 0.95))
            } else {
                Ok(("x".to_string(), 0.20))
            }
        }
    }

    /// A detector that always returns one box the shape of the page, so the
    /// crop it hands over is wide on an upright page and tall on a sideways
    /// one — exactly as a real line of text behaves.
    struct BoxTheWholePage;
    impl Detector for BoxTheWholePage {
        fn detect(&self, image: &GrayImage) -> Result<Vec<Quad>> {
            Ok(vec![Quad::from_rect(
                0.0,
                0.0,
                image.width() as f32,
                image.height() as f32,
            )])
        }
    }

    /// **The fix for the five photographs.** A page that only reads one way up
    /// must be read that way up, and the engine must find that out by reading
    /// rather than by measuring the pixels.
    ///
    /// Falsified by removing the two quarter-turn rungs from the ladder in
    /// `scan_bytes`: this then returns `x` at 0.20 and goes red. It is the only
    /// test that does.
    #[test]
    fn a_page_that_only_reads_sideways_is_turned_until_it_reads() {
        // A tall page, so the detector's box is tall and the stub recogniser
        // refuses it — until the engine turns the page a quarter.
        let mut img = GrayImage::from_pixel(80, 400, ::image::Luma([255u8]));
        for y in 100..300 {
            for x in 20..60 {
                img.put_pixel(x, y, ::image::Luma([0u8]));
            }
        }
        let mut bytes = Vec::new();
        ::image::DynamicImage::ImageLuma8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                ::image::ImageFormat::Png,
            )
            .unwrap();

        let engine = Engine::new(BoxTheWholePage, OnlyReadsUpright).with_options(ScanOptions {
            // Skew estimation on a synthetic block is noise here, and this test
            // is about the quarter turn.
            fix_skew: false,
            ..geometry_only()
        });
        let result = engine.scan_bytes(&bytes).unwrap();
        assert_eq!(
            result.text(),
            "ELEVEN CHAR",
            "the page was never turned; it came back at {:.2} confidence, \
             rotation {}",
            result.confidence(),
            result.rotation
        );
        assert!(result.confidence() > 0.9);
        assert!(!result.needs_review(0.5));
    }

    /// Twenty boxes on a wide page, one on a tall one — the shape a real
    /// detector produces, since a line of type is only a line when the page is
    /// the right way up.
    struct ManyBoxesWhenWide;
    impl Detector for ManyBoxesWhenWide {
        fn detect(&self, image: &GrayImage) -> Result<Vec<Quad>> {
            let (w, h) = image.dimensions();
            if w > h {
                Ok((0..20)
                    .map(|i| Quad::from_rect(0.0, i as f32 * 4.0, w as f32, 3.0))
                    .collect())
            } else {
                Ok(vec![Quad::from_rect(0.0, 0.0, w as f32, h as f32)])
            }
        }
    }

    /// Both readings are poor, and the poorer-looking one is the fuller one.
    struct SureOfNothingMuch;
    impl Recognizer for SureOfNothingMuch {
        fn recognize(&self, crop: &GrayImage) -> Result<(String, f32)> {
            if crop.width() > crop.height() {
                Ok(("TWELVE CHARS".to_string(), 0.45))
            } else {
                Ok(("x".to_string(), 0.49))
            }
        }
    }

    /// **The retry must not prefer an attempt that read almost nothing just
    /// because it was sure of it.**
    ///
    /// Every attempt here is below the floor, which is the regime where the
    /// comparison actually decides something: one box read at 0.49 against
    /// twenty read at 0.45. Mean confidence says the single box is the better
    /// reading. It is not — it is 1 character against 240.
    ///
    /// Falsified by comparing on `again.confidence() > best.confidence()` in
    /// `scan_bytes`, as this did before the ladder grew: the result is then
    /// `x`, and this goes red.
    #[test]
    fn a_confident_sliver_does_not_beat_a_page_that_was_actually_read() {
        let mut img = GrayImage::from_pixel(80, 400, ::image::Luma([255u8]));
        for y in 100..300 {
            for x in 20..60 {
                img.put_pixel(x, y, ::image::Luma([0u8]));
            }
        }
        let mut bytes = Vec::new();
        ::image::DynamicImage::ImageLuma8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                ::image::ImageFormat::Png,
            )
            .unwrap();

        let engine =
            Engine::new(ManyBoxesWhenWide, SureOfNothingMuch).with_options(ScanOptions {
                fix_skew: false,
                ..geometry_only()
            });
        let result = engine.scan_bytes(&bytes).unwrap();
        assert_eq!(
            result.lines.iter().map(|l| l.boxes.len()).sum::<usize>(),
            20,
            "kept the one-character reading; it came back as {:?} at {:.2}",
            result.text(),
            result.confidence(),
        );
        // And the reading it kept is genuinely the less confident one, or the
        // test proves nothing about the comparison.
        assert!(result.confidence() < 0.49);
    }
}
