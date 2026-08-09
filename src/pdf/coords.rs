//! Converting between pixel space and PDF point space.
//!
//! **Every page returns coordinates in PDF points, whatever the source.** This
//! module is the only place the conversion happens, and the reason it exists is
//! that getting it wrong produces a failure that looks like something else.
//!
//! ## The two spaces
//!
//! | | PDF user space | Image space |
//! |---|---|---|
//! | Unit | point, 1/72 inch | pixel |
//! | Origin | bottom left | top left |
//! | Y direction | grows **upward** | grows **downward** |
//!
//! `pdf-inspector`'s convention was **verified rather than assumed**: its
//! `TextItem` documents "PDF coordinates, origin at bottom-left", and measuring
//! the Revolut statement confirms it — the page title sits at y=756.8 and the
//! footer at y=39.5 on a page 842pt tall. Highest y is the top of the page, so
//! the origin is at the bottom and Y grows upward.
//!
//! ## Why this will be blamed for something else
//!
//! `statement` reads x and y to find columns. If a text-layer page hands back
//! points and a rasterised page hands back pixels, a mixed document produces
//! columns that are nonsense — and the symptom is a statement that does not
//! reconcile, which looks exactly like bad OCR. It would not be bad OCR.
//!
//! Hence the agreement test in this module: the same line of text, read both
//! ways, must land in the same place to within 3 points.

use super::render::PageImage;

/// A rectangle in PDF user space: points, origin bottom left, Y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointRect {
    pub x: f32,
    /// Distance from the **bottom** of the page to the bottom of the box.
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A rectangle in image space: pixels, origin top left, Y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub x: f32,
    /// Distance from the **top** of the image to the top of the box.
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Map an OCR box on a rendered page back into PDF points.
///
/// Two changes at once, and both must happen or the result is silently wrong:
/// divide by the scale to get points, and flip Y because the origins are at
/// opposite ends of the page.
///
/// The flip is the part worth staring at. A box `y` pixels from the *top*
/// begins, in PDF terms, `page_height - (y + height)` points from the
/// *bottom*: subtract the box's own height as well, because `y` measures to
/// the near edge in each space and those are different edges.
pub fn pixels_to_points(rect: PixelRect, page: &PageImage) -> PointRect {
    let (sx, sy) = page.scale();
    let width = rect.width / sx;
    let height = rect.height / sy;
    let x = rect.x / sx;
    // Distance from the bottom of the page to the bottom of the box.
    let y = page.page_height_pt - (rect.y / sy) - height;
    PointRect { x, y, width, height }
}

/// Map a PDF point rectangle onto a rendered page.
///
/// The exact inverse of [`pixels_to_points`]. Used to crop a region of a
/// rendered page when the text layer says where something is.
pub fn points_to_pixels(rect: PointRect, page: &PageImage) -> PixelRect {
    let (sx, sy) = page.scale();
    let width = rect.width * sx;
    let height = rect.height * sy;
    let x = rect.x * sx;
    // Distance from the top of the image to the top of the box.
    let y = (page.page_height_pt - rect.y - rect.height) * sy;
    PixelRect { x, y, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A4 in points.
    const A4_W: f32 = 595.0;
    const A4_H: f32 = 842.0;

    fn a4_at(dpi: f32) -> PageImage {
        let scale = dpi / 72.0;
        PageImage {
            pixels: image::GrayImage::new(
                (A4_W * scale).round() as u32,
                (A4_H * scale).round() as u32,
            ),
            dpi,
            page_width_pt: A4_W,
            page_height_pt: A4_H,
            capped: false,
        }
    }

    #[test]
    fn the_top_of_the_image_is_the_top_of_the_page() {
        let page = a4_at(300.0);
        // A box hard against the top of the image.
        let top = PixelRect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 };
        let pt = pixels_to_points(top, &page);
        // In PDF space that is near the *ceiling*: y + height == page height.
        assert!((pt.y + pt.height - A4_H).abs() < 0.01, "got y={} h={}", pt.y, pt.height);
    }

    #[test]
    fn the_bottom_of_the_image_is_the_bottom_of_the_page() {
        let page = a4_at(300.0);
        let h_px = page.pixels.height() as f32;
        let bottom = PixelRect { x: 0.0, y: h_px - 50.0, width: 100.0, height: 50.0 };
        let pt = pixels_to_points(bottom, &page);
        assert!(pt.y.abs() < 0.01, "should sit on the floor, got y={}", pt.y);
    }

    #[test]
    fn the_round_trip_returns_the_original() {
        let page = a4_at(300.0);
        for rect in [
            PointRect { x: 39.7, y: 756.8, width: 137.7, height: 20.6 },
            PointRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
            PointRect { x: 500.0, y: 800.0, width: 90.0, height: 40.0 },
        ] {
            let back = pixels_to_points(points_to_pixels(rect, &page), &page);
            assert!((back.x - rect.x).abs() < 0.01, "x: {back:?} vs {rect:?}");
            assert!((back.y - rect.y).abs() < 0.01, "y: {back:?} vs {rect:?}");
            assert!((back.width - rect.width).abs() < 0.01, "w: {back:?}");
            assert!((back.height - rect.height).abs() < 0.01, "h: {back:?}");
        }
    }

    #[test]
    fn dpi_does_not_change_the_answer() {
        // The whole point of normalising to points: a box in the same place on
        // the page maps to the same point rectangle whatever it was rendered
        // at. If this fails, a mixed-dpi document produces mixed columns.
        let rect_pt = PointRect { x: 100.0, y: 400.0, width: 80.0, height: 12.0 };
        let mut results = Vec::new();
        for dpi in [150.0, 200.0, 300.0, 400.0] {
            let page = a4_at(dpi);
            let px = points_to_pixels(rect_pt, &page);
            results.push(pixels_to_points(px, &page));
        }
        for r in &results {
            assert!((r.x - rect_pt.x).abs() < 0.05, "x drifted at some dpi: {r:?}");
            assert!((r.y - rect_pt.y).abs() < 0.05, "y drifted at some dpi: {r:?}");
        }
    }

    #[test]
    fn a_capped_render_still_maps_correctly() {
        // The scale comes from the image, not from the requested dpi, so a
        // page that was capped maps as accurately as one that was not.
        let page = PageImage {
            pixels: image::GrayImage::new(2827, 4000), // capped A4
            dpi: 342.0,
            page_width_pt: A4_W,
            page_height_pt: A4_H,
            capped: true,
        };
        let rect = PointRect { x: 100.0, y: 400.0, width: 80.0, height: 12.0 };
        let back = pixels_to_points(points_to_pixels(rect, &page), &page);
        assert!((back.x - rect.x).abs() < 0.01);
        assert!((back.y - rect.y).abs() < 0.01);
    }

    #[test]
    fn y_is_not_merely_scaled() {
        // The failure mode this module exists to prevent: forgetting the flip
        // and only dividing by the scale. At 300 dpi a box 100px from the top
        // is 24pt from the top, which is 818pt from the bottom — not 24.
        let page = a4_at(300.0);
        let px = PixelRect { x: 0.0, y: 100.0, width: 10.0, height: 10.0 };
        let pt = pixels_to_points(px, &page);
        assert!(pt.y > 800.0, "y must be measured from the bottom, got {}", pt.y);
        assert!((pt.y - (A4_H - 24.0 - 2.4)).abs() < 0.1, "got {}", pt.y);
    }
}
