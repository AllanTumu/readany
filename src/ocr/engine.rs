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
    /// Correct page rotation of 90, 180 or 270 degrees.
    pub fix_orientation: bool,
    /// Correct small skew introduced by scanners and phone cameras.
    pub fix_skew: bool,
    /// Below this mean confidence, [`ScanResult::needs_review`] returns true.
    pub confidence_floor: f32,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            fix_orientation: true,
            fix_skew: true,
            confidence_floor: 0.5,
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
    let decoded = super::image::decode_bytes(bytes)?;

    let orientation = if options.fix_orientation {
        super::image::orient::detect(&decoded)
    } else {
        super::image::orient::Orientation::Upright
    };
    let turned = super::image::orient::apply(&decoded, orientation);
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
    pub fn scan_bytes(&self, bytes: &[u8]) -> Result<ScanResult> {
        let watch = crate::clock::Stopwatch::start();
        let stage = crate::clock::Stopwatch::start();
        let prepared = prepare_bytes(bytes, &self.options)?;
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
mod tests {
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
        let prepared = prepare_bytes(&png_page(), &ScanOptions::default()).unwrap();
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
        );
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
        let prepared = prepare_bytes(&png_page(), &ScanOptions::default()).unwrap();
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
