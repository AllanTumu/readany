//! Coarse page orientation: is the text upright, or is the page on its side?
//!
//! Upright text produces a spiky horizontal projection and a flat vertical one.
//! Comparing the two tells us whether to rotate by 90 degrees. Distinguishing
//! upright from upside down needs a model, so `Orientation::Upright` and
//! `Orientation::UpsideDown` are only separated once a classifier is wired in.
//!
//! Every buffer here is sized from a decoded image's own dimensions, so the
//! panicking forms are denied. See `docs/security.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::binarize::ink_mask;
use super::deskew::rotate;
use super::GrayImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Upright,
    Rotated90,
    UpsideDown,
    Rotated270,
}

impl Orientation {
    pub fn degrees(self) -> u16 {
        match self {
            Orientation::Upright => 0,
            Orientation::Rotated90 => 90,
            Orientation::UpsideDown => 180,
            Orientation::Rotated270 => 270,
        }
    }
}

/// How much more than the other axis one must spike before the page is called
/// sideways. Not 1.0: a page whose two profiles are near enough equal carries
/// no evidence either way, and turning it on a coin toss is worse than leaving
/// it alone.
const MARGIN: f64 = 1.3;

/// The two numbers [`detect`] decides on.
///
/// Public because the decision was wrong on five real photographs and nothing
/// in the pipeline could say by how much. A verdict with no visible evidence
/// can only be argued about; these can be printed, and were — see
/// `readany-ocr/examples/frame_sizes.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence {
    /// Normalised variance of the per-row ink counts. High when the ink lies in
    /// horizontal bands, which is what upright text is.
    pub across_rows: f64,
    /// Normalised variance of the per-column ink counts. High when the ink lies
    /// in vertical bands: a page on its side.
    pub across_columns: f64,
}

impl Evidence {
    /// The verdict these two numbers support.
    pub fn verdict(self) -> Orientation {
        if self.across_columns > self.across_rows * MARGIN {
            Orientation::Rotated90
        } else {
            Orientation::Upright
        }
    }
}

/// Measure the two projection profiles without deciding anything.
pub fn evidence(img: &GrayImage) -> Evidence {
    let (w, h) = img.dimensions();
    let mask = ink_mask(img);
    Evidence {
        across_rows: axis_variance(&mask, w, h, true),
        across_columns: axis_variance(&mask, w, h, false),
    }
}

/// Decide between upright and sideways using projection spikiness.
///
/// # What this is worth on a photograph: nothing, and it is worse than nothing
///
/// Measured 10 August 2026 on the five photographed receipts in the corpus,
/// 4032x3024 each, every one of them genuinely on its side. The ratio this
/// function tests — columns over rows, sideways above 1.30:
///
/// | Page | as photographed (sideways) | turned upright first |
/// |---|---|---|
/// | 1 | 0.65 | 1.53 |
/// | 2 | 0.48 | 2.07 |
/// | 3 | 0.17 | 6.05 |
/// | 4 | 0.14 | 7.11 |
/// | 5 | 0.26 | 3.90 |
///
/// **Five of five wrong, and wrong the other way round.** It calls a sideways
/// page upright and an upright page sideways, confidently, on real input.
///
/// The reason is visible the moment the mask is saved and looked at, which is
/// how it was found. [`ink_mask`] is a global Otsu threshold, so on a
/// photograph the largest dark things it keeps are not the print — they are the
/// shadow under the curled paper and the dark background beyond its edge. Those
/// blobs run the width of the frame and contribute thousands of pixels to a few
/// hundred rows, which is a far bigger row-to-row variation than a line of
/// 4-point type. The projection is then measuring where the shadow is, not
/// which way the text runs. Substituting a local-contrast mask for the Otsu one
/// was tried and measured: it gets three of the five, and stays inverted on the
/// two whose paper edge runs straight across the frame.
///
/// It is kept because it is measured to work on the case it was built for — a
/// fed or photocopied page, where the sheet is the whole frame and there is no
/// shadow to out-vote the text. On a **scan** tilted 270 degrees this is what
/// recovers 18 of 18 rows; see [`crate::ocr::ScanOptions::for_scanned_page`].
///
/// A photograph's orientation is settled downstream instead, by reading the
/// page each way up and keeping the attempt that read — see
/// [`crate::ocr::ScanOptions::turn`]. Replacing this with a real orientation
/// classifier (PaddleOCR ships a 0.6 MB one) would let that retry be skipped,
/// and is the right eventual fix; guessing at a better projection statistic is
/// not.
pub fn detect(img: &GrayImage) -> Orientation {
    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return Orientation::Upright;
    }
    evidence(img).verdict()
}

fn axis_variance(mask: &[bool], w: u32, h: u32, by_row: bool) -> f64 {
    // `chunks` panics on a zero width, and a zero-sided page has no variance to
    // measure anyway.
    if w == 0 || h == 0 {
        return 0.0;
    }
    let n = if by_row { h } else { w } as usize;
    let mut counts = vec![0u32; n];
    // Walking the mask a row at a time replaces the flattened `y * w + x`, so
    // there is no offset left to get wrong and no bound left to re-derive.
    for (y, row) in mask.chunks(w as usize).enumerate().take(h as usize) {
        for (x, &inked) in row.iter().enumerate() {
            if inked {
                if let Some(count) = counts.get_mut(if by_row { y } else { x }) {
                    *count = count.saturating_add(1);
                }
            }
        }
    }
    let mean = counts.iter().map(|&c| c as f64).sum::<f64>() / n as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let var = counts
        .iter()
        .map(|&c| {
            let d = c as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n as f64;
    // Normalise so page size does not dominate.
    var / (mean * mean)
}

/// Rotate the image so the text runs left to right.
pub fn apply(img: &GrayImage, orientation: Orientation) -> GrayImage {
    match orientation {
        Orientation::Upright => img.clone(),
        Orientation::Rotated90 => rotate_quarter(img, true),
        Orientation::Rotated270 => rotate_quarter(img, false),
        Orientation::UpsideDown => rotate(img, 180.0),
    }
}

fn rotate_quarter(img: &GrayImage, clockwise: bool) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::new(h, w);
    // Zipping each axis against its own reverse gives `h - 1 - y` and
    // `w - 1 - x` without subtracting, so neither can be made to wrap.
    for (y, mirrored_y) in (0..h).zip((0..h).rev()) {
        for (x, mirrored_x) in (0..w).zip((0..w).rev()) {
            let p = *img.get_pixel(x, y);
            if clockwise {
                out.put_pixel(mirrored_y, x, p);
            } else {
                out.put_pixel(y, mirrored_x, p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // See the note on the same allow in `route`.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;

    fn upright_text() -> GrayImage {
        let mut img = GrayImage::from_pixel(120, 120, image::Luma([255u8]));
        for band in 0..8 {
            let y0 = 10 + band * 13;
            for y in y0..y0 + 4 {
                for x in 15..105 {
                    img.put_pixel(x, y, image::Luma([0u8]));
                }
            }
        }
        img
    }

    #[test]
    fn upright_page_is_recognised() {
        assert_eq!(detect(&upright_text()), Orientation::Upright);
    }

    #[test]
    fn sideways_page_is_recognised() {
        let sideways = rotate_quarter(&upright_text(), true);
        assert_eq!(detect(&sideways), Orientation::Rotated90);
    }

    #[test]
    fn quarter_turns_swap_dimensions() {
        let img = upright_text();
        let out = apply(&img, Orientation::Rotated90);
        assert_eq!((out.width(), out.height()), (img.height(), img.width()));
    }
}
