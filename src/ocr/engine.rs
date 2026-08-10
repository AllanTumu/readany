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
    /// A phone photograph of a receipt is mostly table, and the detector
    /// shrinks whatever it is given to a fixed working size, so every glyph
    /// shrinks with the table. Cutting the document out first is worth more
    /// than any other single change measured on real photographs.
    pub crop_to_content: bool,
    /// Correct page rotation of 90, 180 or 270 degrees.
    pub fix_orientation: bool,
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
            fix_skew: false,
            confidence_floor: 0.5,
            // Nothing to retry *with*: the corrections that a retry turns off
            // are already off.
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
    let whole = super::image::decode_bytes(bytes)?;

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

    let orientation = if options.fix_orientation {
        super::image::orient::detect(&evened)
    } else {
        super::image::orient::Orientation::Upright
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
}

impl<D: Detector, R: Recognizer> Engine<D, R> {
    pub fn new(detector: D, recognizer: R) -> Self {
        Engine {
            detector,
            recognizer,
            options: ScanOptions::default(),
        }
    }

    pub fn with_options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Read a page: straighten, detect, recognise, order.
    ///
    /// If the first attempt comes back unsure and `retry_when_unsure` is set,
    /// the page is read again without the geometric corrections and the better
    /// attempt is returned. A wrongly detected rotation is the commonest way a
    /// perfectly readable page turns into nonsense, and the engine can tell
    /// that it happened, so it should not hand the nonsense back.
    pub fn scan_bytes(&self, bytes: &[u8]) -> Result<ScanResult> {
        let mut best = self.scan_once(bytes, &self.options)?;
        if !self.options.retry_when_unsure || best.confidence() >= self.options.confidence_floor {
            return Ok(best);
        }

        for attempt in [
            ScanOptions {
                fix_orientation: false,
                ..self.options.clone()
            },
            ScanOptions {
                fix_orientation: false,
                fix_skew: false,
                ..self.options.clone()
            },
        ] {
            let again = self.scan_once(bytes, &attempt)?;
            if again.confidence() > best.confidence() {
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
        let prepared = prepare_bytes(bytes, options)?;
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
}
