//! The rasterisation seam: a trait, and nothing that implements it.
//!
//! A PDF page with no text layer is a picture. The OCR engine reads pictures.
//! This is the missing pipe between them — declared here, implemented
//! elsewhere.
//!
//! ## Why the implementation is not in this crate
//!
//! `readany` ships to npm as WebAssembly. PDFium is native C++ and can never
//! enter it. That is the same split already used for OCR: [`crate::OcrBackend`]
//! is declared here and implemented in `readany-ocr`, which holds ONNX.
//!
//! This module mirrors it exactly. `readany-pdfium` is the only place PDFium is
//! named, and a build of this crate for `wasm32-unknown-unknown` never sees it.
//!
//! ## Why a trait rather than a function
//!
//! Three renderers will exist and they are not interchangeable at build time:
//!
//! | Where | Renderer |
//! |---|---|
//! | Server (Hetzner) | PDFium, through `readany-pdfium` |
//! | Android | `android.graphics.pdf.PdfRenderer`, in the platform |
//! | iOS | PDFKit, in the platform |
//!
//! Every one of them can answer "how many pages" and "give me page N as a
//! grey image at D dots per inch". None of them should be asked anything else
//! through this trait — a method only PDFium can answer would silently make the
//! seam PDFium-shaped, and the phone implementations would have to fake it.

use crate::error::Result;

/// One rendered page.
#[derive(Debug, Clone)]
pub struct PageImage {
    /// Greyscale pixels. Grey rather than colour because OCR discards colour
    /// anyway, and an A4 page at 300 dpi is 8.7 MB grey against 26 MB in RGB.
    pub pixels: image::GrayImage,
    /// Dots per inch this page was actually rendered at.
    ///
    /// May be lower than the dpi requested: see [`PageImage::capped`].
    pub dpi: f32,
    /// Page width in PDF points, so callers can map pixels back to the
    /// coordinate space the text layer uses.
    pub page_width_pt: f32,
    /// Page height in PDF points. Needed for the Y flip — see
    /// [`crate::pdf::coords`].
    pub page_height_pt: f32,
    /// True when the requested dpi was reduced to keep the image within the
    /// size cap. Stated rather than silent: a caller that asked for 400 dpi
    /// and received 260 needs to know before it blames the OCR.
    pub capped: bool,
}

impl PageImage {
    /// Pixels per PDF point, in each axis.
    ///
    /// Not `dpi / 72` — a renderer may round the pixel dimensions, so the true
    /// scale is whatever the image and the page actually are. Deriving it from
    /// the two makes the coordinate transform exact rather than nearly right.
    pub fn scale(&self) -> (f32, f32) {
        (
            self.pixels.width() as f32 / self.page_width_pt,
            self.pixels.height() as f32 / self.page_height_pt,
        )
    }
}

/// Turn PDF pages into pictures.
///
/// Deliberately two methods. Anything else belongs to the renderer's own crate,
/// not to this seam.
pub trait Rasterise {
    /// How many pages does this PDF have?
    fn page_count(&self, pdf: &[u8]) -> Result<usize>;

    /// Render one page, 0-indexed, at the requested dots per inch.
    ///
    /// Implementations must:
    ///
    /// - convert to grey *inside* the renderer, never handing back RGB for the
    ///   caller to convert — that triples peak memory for no gain;
    /// - cap the longest side at [`MAX_SIDE_PX`] whatever dpi was asked for,
    ///   and set [`PageImage::capped`] when they do;
    /// - render one page at a time, holding no others.
    fn render_page(&self, pdf: &[u8], page: usize, dpi: f32) -> Result<PageImage>;

    /// Render one page of a password-protected PDF.
    ///
    /// Many bank statements are encrypted with an empty owner password, and
    /// some with the customer's date of birth. The default implementation
    /// ignores the password and calls [`Rasterise::render_page`], which is
    /// correct for the empty-password case that a renderer handles silently.
    ///
    /// A renderer that needs a password and was not given a usable one must
    /// return [`crate::ReadError::PasswordRequired`] rather than guessing.
    /// Guessing an account number would be worse than failing.
    fn render_page_with_password(
        &self,
        pdf: &[u8],
        page: usize,
        dpi: f32,
        _password: Option<&str>,
    ) -> Result<PageImage> {
        self.render_page(pdf, page, dpi)
    }
}

/// The longest side any rendered page may have, in pixels.
///
/// An A4 page at 300 dpi is 2480 × 3508, which is under this. At 600 dpi it
/// would be 4960 × 7016 and 35 MB in grey, which is not a page size any OCR
/// engine benefits from — the detector downsamples it again immediately.
pub const MAX_SIDE_PX: u32 = 4000;

/// The dpi to render at when the caller expresses no preference.
///
/// **150, not 300.** Measured on a rendered CaixaBank page: 150 and 400 dpi
/// recover the identical 68 lines and 44 lines-with-digits, and confidence
/// moves only 0.991 to 0.994. Recall is saturated well below 300.
///
/// So 150 is as accurate, 18% faster, and a quarter of the memory — 2.1 MB a
/// page against 8.3 MB. See `docs/ocr-dpi-sweep.md`.
///
/// This is a *rendered page* default. A photograph is a different problem and
/// its resolution is whatever the camera gave.
pub const DEFAULT_DPI: f32 = 150.0;

/// Reduce a requested dpi so the rendered page fits [`MAX_SIDE_PX`].
///
/// Returns the dpi to use and whether it was reduced.
pub fn cap_dpi(page_width_pt: f32, page_height_pt: f32, requested: f32) -> (f32, bool) {
    let longest_pt = page_width_pt.max(page_height_pt);
    if longest_pt <= 0.0 {
        return (requested, false);
    }
    let max_dpi = MAX_SIDE_PX as f32 * 72.0 / longest_pt;
    if requested > max_dpi {
        (max_dpi, true)
    } else {
        (requested, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A4 in points.
    const A4_W: f32 = 595.0;
    const A4_H: f32 = 842.0;

    #[test]
    fn a4_at_300_dpi_is_not_capped() {
        let (dpi, capped) = cap_dpi(A4_W, A4_H, 300.0);
        assert_eq!(dpi, 300.0);
        assert!(!capped);
        // 842pt / 72 * 300 = 3508px, comfortably under the cap.
        assert!((A4_H / 72.0 * dpi) < MAX_SIDE_PX as f32);
    }

    #[test]
    fn an_absurd_dpi_is_capped_and_says_so() {
        let (dpi, capped) = cap_dpi(A4_W, A4_H, 1200.0);
        assert!(capped, "1200 dpi on A4 would be 14033px");
        assert!(dpi < 1200.0);
        // And the cap is exact: the longest side lands on the limit.
        let px = A4_H / 72.0 * dpi;
        assert!((px - MAX_SIDE_PX as f32).abs() < 1.0, "got {px}px");
    }

    #[test]
    fn a_degenerate_page_does_not_divide_by_zero() {
        let (dpi, capped) = cap_dpi(0.0, 0.0, 300.0);
        assert_eq!(dpi, 300.0);
        assert!(!capped);
    }

    #[test]
    fn scale_comes_from_the_image_not_from_the_dpi() {
        // A renderer that rounds 2479.7px to 2480 leaves the true scale
        // slightly off dpi/72. Deriving it from the image keeps the transform
        // exact.
        let img = image::GrayImage::new(2480, 3508);
        let page = PageImage {
            pixels: img,
            dpi: 300.0,
            page_width_pt: A4_W,
            page_height_pt: A4_H,
            capped: false,
        };
        let (sx, sy) = page.scale();
        assert!((sx - 2480.0 / A4_W).abs() < 1e-6);
        assert!((sy - 3508.0 / A4_H).abs() < 1e-6);
        // And it is close to, but not exactly, the nominal scale.
        assert!((sx - 300.0 / 72.0).abs() < 0.01);
    }
}
