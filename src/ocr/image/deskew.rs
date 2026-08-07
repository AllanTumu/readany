//! Skew correction by projection profile.
//!
//! Text lines make the horizontal ink projection spiky. Rotating the page to
//! the angle that maximises the variance of that projection straightens it.
//! This is cheap, needs no model, and handles the +/- 15 degrees that scanners
//! and phone photos usually introduce.

use super::binarize::ink_mask;
use super::GrayImage;

const SEARCH_DEGREES: f32 = 15.0;
const COARSE_STEP: f32 = 1.0;
const FINE_STEP: f32 = 0.1;
/// How much sharper the straightened projection must be before we rotate.
const MIN_PEAK_GAIN: f64 = 1.05;

/// Estimate the page skew in degrees. Positive means the page is rotated
/// clockwise and must be rotated back by that amount.
pub fn estimate_skew(img: &GrayImage) -> f32 {
    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return 0.0;
    }
    let mask = ink_mask(img);

    let (coarse, coarse_score) =
        best_angle(&mask, w, h, -SEARCH_DEGREES, SEARCH_DEGREES, COARSE_STEP);

    // A best angle sitting on the edge of the search is not an answer, it is
    // the search running out of room. It happens on creased and curved paper,
    // where the ink projection has no clear peak, and acting on it rotates a
    // perfectly readable page into nonsense.
    //
    // Measured on a creased receipt: the estimate saturated at 15.9 degrees
    // against a true tilt of about 3, and correcting it took field extraction
    // from 6 of 8 down to 2 of 8. Returning zero here is not giving up; it is
    // declining to make the page worse.
    if coarse.abs() >= SEARCH_DEGREES - COARSE_STEP / 2.0 {
        return 0.0;
    }

    // The peak must also be a real peak. If straightening barely beats leaving
    // the page alone, leave it alone.
    let flat_score = projection_variance(&mask, w, h, 0.0);
    if coarse_score < flat_score * MIN_PEAK_GAIN {
        return 0.0;
    }

    let (fine, _) = best_angle(
        &mask,
        w,
        h,
        coarse - COARSE_STEP,
        coarse + COARSE_STEP,
        FINE_STEP,
    );
    fine
}

/// Returns the best angle and its score, so the caller can judge whether the
/// peak is worth acting on.
fn best_angle(mask: &[bool], w: u32, h: u32, from: f32, to: f32, step: f32) -> (f32, f64) {
    let mut best = 0.0f32;
    let mut best_score = f64::MIN;
    let mut angle = from;
    while angle <= to {
        let score = projection_variance(mask, w, h, angle);
        if score > best_score {
            best_score = score;
            best = angle;
        }
        angle += step;
    }
    (best, best_score)
}

/// Variance of the row-wise ink counts after a shear of `angle` degrees.
/// A shear approximates a small rotation and costs one multiply per pixel.
fn projection_variance(mask: &[bool], w: u32, h: u32, angle: f32) -> f64 {
    let slope = (-angle).to_radians().tan();
    let mut rows = vec![0u32; h as usize];

    for y in 0..h {
        let row_start = (y as usize) * (w as usize);
        for x in 0..w {
            if !mask[row_start + x as usize] {
                continue;
            }
            let shifted = y as f32 + slope * (x as f32 - w as f32 / 2.0);
            if shifted >= 0.0 && shifted < h as f32 {
                rows[shifted as usize] += 1;
            }
        }
    }

    let n = rows.len() as f64;
    let mean = rows.iter().map(|&c| c as f64).sum::<f64>() / n;
    rows.iter()
        .map(|&c| {
            let d = c as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n
}

/// Rotate an image about its centre by `degrees`, with bilinear sampling.
/// Pixels outside the source are filled with white.
pub fn rotate(img: &GrayImage, degrees: f32) -> GrayImage {
    if degrees.abs() < 0.01 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let theta = degrees.to_radians();
    let (sin, cos) = theta.sin_cos();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);

    let mut out = GrayImage::from_pixel(w, h, image::Luma([255u8]));

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            // Inverse rotation: where did this output pixel come from?
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
                continue;
            }
            out.put_pixel(x, y, image::Luma([bilinear(img, sx, sy)]));
        }
    }
    out
}

fn bilinear(img: &GrayImage, x: f32, y: f32) -> u8 {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let p = |px: u32, py: u32| img.get_pixel(px, py).0[0] as f32;
    let top = p(x0, y0) * (1.0 - fx) + p(x0 + 1, y0) * fx;
    let bottom = p(x0, y0 + 1) * (1.0 - fx) + p(x0 + 1, y0 + 1) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a page of horizontal text-like bars.
    fn ruled_page() -> GrayImage {
        let mut img = GrayImage::from_pixel(120, 120, image::Luma([255u8]));
        for band in 0..8 {
            let y0 = 10 + band * 13;
            for y in y0..y0 + 5 {
                for x in 15..105 {
                    img.put_pixel(x, y, image::Luma([0u8]));
                }
            }
        }
        img
    }

    #[test]
    fn straight_page_has_near_zero_skew() {
        let skew = estimate_skew(&ruled_page());
        assert!(skew.abs() < 1.0, "skew was {skew}");
    }

    #[test]
    fn skewed_page_is_detected_and_corrected() {
        let rotated = rotate(&ruled_page(), 5.0);
        let skew = estimate_skew(&rotated);
        assert!(
            (skew - 5.0).abs() < 1.5,
            "expected about 5 degrees, measured {skew}"
        );
        let fixed = rotate(&rotated, -skew);
        assert!(estimate_skew(&fixed).abs() < 1.5);
    }

    #[test]
    fn rotation_preserves_dimensions() {
        let img = ruled_page();
        let out = rotate(&img, 7.0);
        assert_eq!(img.dimensions(), out.dimensions());
    }

    /// A page with no clear text lines — creased paper, a photograph of a
    /// crumpled receipt — used to drive the search to its boundary and rotate
    /// the page by 15.9 degrees, which destroyed text that was readable.
    #[test]
    fn a_saturated_estimate_is_refused() {
        // Diagonal ink with no horizontal structure: every angle scores about
        // the same, so the search has no peak to find.
        let mut img = GrayImage::from_pixel(200, 200, image::Luma([255u8]));
        for i in 0..200u32 {
            img.put_pixel(i, i, image::Luma([0]));
            if i + 1 < 200 {
                img.put_pixel(i + 1, i, image::Luma([0]));
            }
        }
        let skew = estimate_skew(&img);
        assert!(
            skew.abs() < SEARCH_DEGREES - COARSE_STEP,
            "an estimate at the edge of the search must be refused, got {skew}"
        );
    }

    /// Blank paper has nothing to straighten and must not be rotated.
    #[test]
    fn a_blank_page_is_left_alone() {
        let img = GrayImage::from_pixel(200, 200, image::Luma([255u8]));
        assert_eq!(estimate_skew(&img), 0.0);
    }
}
