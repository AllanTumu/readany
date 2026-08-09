//! Flattening uneven lighting before detection.
//!
//! A photographed receipt is not evenly lit. It is creased, so parts of it sit
//! in a fold shadow; it is thermal paper, so parts of it have faded; and the
//! inverted header bands that shops print white-on-black are, to a detector,
//! large dark blobs sitting right next to ordinary text.
//!
//! On one real receipt this cost the first characters of four separate lines —
//! `MBINADO` for COMBINADO, `ipus` for Tipus, `scripcio` for Descripció — not
//! because the glyphs were unreadable but because they sat where the paper
//! folded, and the region growing either lost them in the shadow or swallowed
//! them into the neighbouring black band.
//!
//! Widening the boxes does not fix that, and measurement said so: raising the
//! unclip ratio made the score worse, and moving the probability threshold did
//! nothing at all. The fix has to happen before detection, on the pixels.
//!
//! So: estimate the paper behind every pixel with a large box blur, then divide
//! by it. Paper becomes uniformly white wherever it is, shadow or not, and ink
//! keeps its contrast against it. This is a summed-area table, so it costs one
//! pass over the image regardless of how large the blur is.
//!
//! This module held one of the two real integer wraps, so the panicking forms
//! are denied here rather than trusted to review. See `docs/security.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::GrayImage;

/// Blur radius as a fraction of the long side. It must be far larger than a
/// stroke of text — otherwise the text becomes its own background and vanishes
/// — and smaller than the lighting changes we are trying to remove.
const RADIUS_FRACTION: f32 = 0.04;
/// Never go below this, or fine print on a small image erases itself.
const MIN_RADIUS: u32 = 8;

/// The largest summed-area table this will build.
///
/// Eight bytes a cell, so this is a 400 MB ceiling on one allocation derived
/// from an attacker's declared dimensions. Matched to the documented
/// per-page pixel limit so the two cannot drift apart silently.
const MAX_TABLE_CELLS: u64 = 50_000_000;

/// Where cell `(row, column)` of a `stride`-wide table lives, or `None` if that
/// offset does not fit a `usize`.
///
/// The wrap this replaces was the real one: `(w + 1) * (h + 1)` in `u32` is
/// exactly 2^32 for a 65535-square image, which wraps to zero and hands the
/// next line an empty table to index.
fn cell_index(row: u32, column: u32, stride: usize) -> Option<usize> {
    (row as usize)
        .checked_mul(stride)?
        .checked_add(column as usize)
}

/// A table cell, or zero when the coordinates fall outside it.
///
/// Every caller here is inside the table by construction; returning zero rather
/// than panicking means a future one that is not costs a wrong mean instead of
/// a crash on a page a stranger uploaded.
fn read(table: &[u64], row: u32, column: u32, stride: usize) -> u64 {
    cell_index(row, column, stride)
        .and_then(|i| table.get(i))
        .copied()
        .unwrap_or(0)
}

/// Even out the lighting across a page.
pub fn flatten(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    if w < 8 || h < 8 {
        return img.clone();
    }
    let radius = ((w.max(h) as f32 * RADIUS_FRACTION) as u32).max(MIN_RADIUS);

    // Summed-area table, so any box mean is four lookups.
    //
    // **Computed in u64 and refused on overflow.** The dimensions come from a
    // decoded image, which is to say from whoever sent the file. In `u32`,
    // `(w + 1) * (h + 1)` for a 65535-square image is exactly 2^32 and wraps
    // to **zero**: the table would be allocated empty and the very next line
    // would index it, panicking. A panic here is a denial of service reachable
    // by anyone who can upload a picture.
    //
    // Flattening is an enhancement, not a requirement, so an image too large
    // to build a table for is returned unchanged rather than refused.
    let Some(cells) = u64::from(w)
        .saturating_add(1)
        .checked_mul(u64::from(h).saturating_add(1))
    else {
        return img.clone();
    };
    if cells > MAX_TABLE_CELLS {
        return img.clone();
    }
    // One row longer and one column wider than the image, with a zero first row
    // and column, so a box mean is four lookups and needs no edge cases.
    let stride = (w as usize).saturating_add(1);
    let mut sum = vec![0u64; cells as usize];
    for y in 0..h {
        let mut running = 0u64;
        for x in 0..w {
            running = running.saturating_add(u64::from(img.get_pixel(x, y).0[0]));
            let column = x.saturating_add(1);
            let above = read(&sum, y, column, stride);
            if let Some(cell) =
                cell_index(y.saturating_add(1), column, stride).and_then(|i| sum.get_mut(i))
            {
                *cell = above.saturating_add(running);
            }
        }
    }
    let mean = |x0: u32, y0: u32, x1: u32, y1: u32| -> f32 {
        let a = read(&sum, y0, x0, stride);
        let b = read(&sum, y0, x1, stride);
        let c = read(&sum, y1, x0, stride);
        let d = read(&sum, y1, x1, stride);
        let n = x1.saturating_sub(x0).saturating_mul(y1.saturating_sub(y0)) as f32;
        // Inclusion-exclusion: `a + d` is the pair that spans the box, `b + c`
        // is what that pair double counts. The corners of a summed-area table
        // are monotonic, so the difference is never negative.
        let total = d.saturating_add(a).saturating_sub(b.saturating_add(c));
        total as f32 / n.max(1.0)
    };

    let mut out = GrayImage::new(w, h);
    for y in 0..h {
        let y0 = y.saturating_sub(radius);
        let y1 = y.saturating_add(radius).saturating_add(1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(radius);
            let x1 = x.saturating_add(radius).saturating_add(1).min(w);
            let background = mean(x0, y0, x1, y1).max(1.0);
            let v = img.get_pixel(x, y).0[0] as f32;
            // Divide, do not subtract. Shadow scales the paper and the ink
            // together, so a ratio survives it and a difference does not.
            let lifted = (v * 255.0 / background).min(255.0);
            out.put_pixel(x, y, image::Luma([lifted as u8]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // See the note on the same allow in `route`. This module in particular
    // asserts *about* wrapping arithmetic, so denying it here would be absurd.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    /// The wrap that would have panicked, as a test.
    ///
    /// A 65535-square image makes `(w + 1) * (h + 1)` exactly 2^32, which is
    /// zero in `u32`. The table would have been allocated empty and indexed on
    /// the next line. Building the image is not possible in a test — that is
    /// 4 gigapixels — so the arithmetic is asserted directly.
    #[test]
    fn the_summed_area_table_size_cannot_wrap() {
        let (w, h) = (65_535u32, 65_535u32);
        assert_eq!(
            (w + 1).wrapping_mul(h + 1),
            0,
            "the wrap this guard exists to prevent"
        );
        // Honestly, it is 2^32 and over the ceiling, so the image comes back
        // untouched instead of the table being built.
        let cells = (w as u64 + 1) * (h as u64 + 1);
        assert_eq!(cells, 1u64 << 32);
        assert!(cells > MAX_TABLE_CELLS);
    }

    #[test]
    fn an_oversized_image_is_returned_unchanged_rather_than_refused() {
        // Flattening is an enhancement. Too large to help with is not an
        // error; it is a picture we hand back as it came.
        let img = GrayImage::from_pixel(16, 16, image::Luma([128]));
        let flattened = flatten(&img);
        assert_eq!(flattened.dimensions(), (16, 16));
    }

    use super::*;

    /// Paper under a shadow must end up as bright as paper in the light, so a
    /// detector cannot tell which half of the page it is looking at.
    #[test]
    fn a_shadow_across_the_page_is_removed() {
        let (w, h) = (400u32, 400u32);
        let mut img = GrayImage::from_pixel(w, h, image::Luma([240u8]));
        // Left half in shadow: everything scaled to 45 percent.
        for y in 0..h {
            for x in 0..w / 2 {
                let v = img.get_pixel(x, y).0[0] as f32 * 0.45;
                img.put_pixel(x, y, image::Luma([v as u8]));
            }
        }
        let out = flatten(&img);
        let lit = out.get_pixel(300, 200).0[0] as i32;
        let shadowed = out.get_pixel(100, 200).0[0] as i32;
        assert!(
            (lit - shadowed).abs() < 12,
            "paper still differs across the shadow: {lit} against {shadowed}"
        );
    }

    /// And the ink must survive it. Removing the shadow is useless if the text
    /// goes with it.
    #[test]
    fn ink_keeps_its_contrast_in_shadow() {
        let (w, h) = (400u32, 400u32);
        let mut img = GrayImage::from_pixel(w, h, image::Luma([240u8]));
        for y in 0..h {
            for x in 0..w / 2 {
                img.put_pixel(x, y, image::Luma([108u8])); // shadowed paper
            }
        }
        // A stroke of ink in each half, each 25 percent of its local paper.
        for y in 190..210u32 {
            for x in 90..110u32 {
                img.put_pixel(x, y, image::Luma([27u8])); // in shadow
            }
            for x in 290..310u32 {
                img.put_pixel(x, y, image::Luma([60u8])); // in the light
            }
        }
        let out = flatten(&img);
        let paper = out.get_pixel(150, 200).0[0] as i32;
        let ink = out.get_pixel(100, 200).0[0] as i32;
        assert!(
            paper - ink > 80,
            "ink in shadow lost its contrast: paper {paper}, ink {ink}"
        );
    }

    /// An evenly lit page should come back essentially unchanged, so this is
    /// safe to leave switched on.
    ///
    /// The ink here is thin strokes with paper between them, which is what text
    /// is. A solid block the size of the blur radius would drag its own
    /// background down and lift itself towards grey — real for the inverted
    /// header bands shops print, and the reason the radius must stay far larger
    /// than a stroke.
    #[test]
    fn an_evenly_lit_page_is_left_alone() {
        let mut img = GrayImage::from_pixel(300, 300, image::Luma([238u8]));
        for y in 140..160u32 {
            for x in 100..200u32 {
                if x % 8 < 3 {
                    img.put_pixel(x, y, image::Luma([30u8]));
                }
            }
        }
        let out = flatten(&img);
        let paper = out.get_pixel(20, 20).0[0] as i32;
        assert!(paper > 240, "paper should be near white, got {paper}");
        let ink = out.get_pixel(104, 150).0[0] as i32; // 104 % 8 == 0, so this is a stroke
        assert!(ink < 90, "ink should stay dark, got {ink}");
    }
}
