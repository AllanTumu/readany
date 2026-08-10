//! Finding the document inside a photograph.
//!
//! A phone photograph of a receipt can be mostly table. The detector shrinks
//! the whole frame to its working size, so the receipt — and every glyph on it
//! — shrinks with it.
//!
//! # What the real photographs looked like
//!
//! Measured 10 August 2026 on the five in the corpus, 4032 × 3024 each, by
//! saving the image this module hands on and looking at it: the receipt fills
//! **61% to 100%** of the frame on all five. Two were above [`MAX_AREA`] and
//! this module correctly returned `None`; the other three were trimmed by 12 to
//! 39%. There is no small object on a large table anywhere in the set.
//!
//! That is the module working, not failing, and it is worth writing down
//! because the sentence above was used to explain a defect it had nothing to do
//! with. The five photographs came back at one character a box, and the cause
//! was orientation — see [`super::orient::detect`].
//!
//! **What is still unmeasured here**: a receipt that genuinely is a small
//! object in a large frame. Nobody in this corpus took that photograph, so the
//! claim that cropping is "worth more than any other single change" rests on no
//! measurement in this repository and has been withdrawn from
//! [`crate::ocr::ScanOptions::crop_to_content`]. The synthetic tests below
//! cover the shape; a real one would need a photograph nobody has taken yet.
//!
//! Edge detection is the obvious approach and it is the wrong one here. A white
//! receipt on a pale table has almost no edge to find, and the three real
//! receipts that started this work were all white on white.
//!
//! So this keys on **texture** instead. Print produces sharp variation over a
//! few pixels. Tables, shadows and paper do not: a shadow is a smooth gradient,
//! and smooth means low contrast inside a small window however dark it gets.
//! Measuring contrast in an 8-pixel cell therefore finds ink and ignores both
//! the background and the lighting.
//!
//! Every buffer here is sized from a decoded image's own dimensions, so the
//! panicking forms are denied. See `docs/security.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::GrayImage;

/// Long side of the small copy this works on. The whole point is that this is
/// cheap, and ink is just as findable at 800 pixels as at 4000.
const WORK_SIDE: u32 = 800;
/// Cell side, in pixels of the small copy.
const CELL: u32 = 8;
/// Light-to-dark range inside one cell before we call it ink.
const INK_CONTRAST: u8 = 40;
/// Cells to grow the mask by before grouping, so the blank gaps between lines
/// of text do not split one receipt into thirty separate findings.
const BRIDGE: i32 = 2;
/// Padding added to the result, as a fraction of the long side.
const MARGIN: f32 = 0.02;
/// Below this share of the frame the finding is noise, not a document.
const MIN_AREA: f32 = 0.02;
/// Above this share there is nothing to crop and we should not pretend.
const MAX_AREA: f32 = 0.92;

/// Where cell `(cx, cy)` lives in a `cols`-wide mask, or `None` when that
/// offset does not fit a `usize`.
fn cell_index(cx: u32, cy: u32, cols: u32) -> Option<usize> {
    (cy as usize)
        .checked_mul(cols as usize)?
        .checked_add(cx as usize)
}

/// Is cell `(cx, cy)` set? Cells outside the mask read as unset, which is what
/// every caller here wants at a border.
fn cell(mask: &[bool], cx: u32, cy: u32, cols: u32) -> bool {
    cell_index(cx, cy, cols)
        .and_then(|i| mask.get(i))
        .copied()
        .unwrap_or(false)
}

/// Step `delta` cells from `at`, or `None` if that leaves `0..limit`.
///
/// This replaces casting the whole grow loop through `i32` to make a negative
/// neighbour representable. The bounds check and the sign live in one place.
fn offset(at: u32, delta: i32, limit: u32) -> Option<u32> {
    let moved = u32::try_from((at as i64).checked_add(delta as i64)?).ok()?;
    (moved < limit).then_some(moved)
}

/// Where the document is, as `(x, y, width, height)` in the image's own
/// pixels. `None` means nothing document-shaped was found and the caller
/// should use the whole frame rather than guess.
pub fn content_bounds(img: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = img.dimensions();
    if w < CELL * 4 || h < CELL * 4 {
        return None;
    }

    // Work small. A 12 megapixel photograph becomes about half a megapixel.
    let scale = (WORK_SIDE as f32 / w.max(h) as f32).min(1.0);
    let sw = ((w as f32 * scale) as u32).max(CELL * 4);
    let sh = ((h as f32 * scale) as u32).max(CELL * 4);
    let small = image::imageops::resize(img, sw, sh, image::imageops::FilterType::Triangle);

    let (cols, rows) = (sw / CELL, sh / CELL);
    if cols < 4 || rows < 4 {
        return None;
    }

    // One pass: does this cell contain ink?
    let mut ink = vec![false; (cols as usize).saturating_mul(rows as usize)];
    for cy in 0..rows {
        let y_start = cy.saturating_mul(CELL);
        for cx in 0..cols {
            let x_start = cx.saturating_mul(CELL);
            let (mut lo, mut hi) = (255u8, 0u8);
            for y in y_start..y_start.saturating_add(CELL) {
                for x in x_start..x_start.saturating_add(CELL) {
                    let v = small.get_pixel(x, y).0[0];
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            if let Some(slot) = cell_index(cx, cy, cols).and_then(|i| ink.get_mut(i)) {
                *slot = hi.saturating_sub(lo) >= INK_CONTRAST;
            }
        }
    }

    // Grow, so one receipt is one region rather than one region per line.
    let mut grown = vec![false; ink.len()];
    for cy in 0..rows {
        for cx in 0..cols {
            let near_ink = (-BRIDGE..=BRIDGE).any(|dy| {
                (-BRIDGE..=BRIDGE).any(|dx| {
                    match (offset(cx, dx, cols), offset(cy, dy, rows)) {
                        (Some(nx), Some(ny)) => cell(&ink, nx, ny, cols),
                        _ => false,
                    }
                })
            });
            if near_ink {
                if let Some(slot) = cell_index(cx, cy, cols).and_then(|i| grown.get_mut(i)) {
                    *slot = true;
                }
            }
        }
    }

    // The biggest group of grown cells is the document. A second receipt in
    // the frame, or a dark object at the edge, becomes a smaller group and is
    // ignored — which is the behaviour we want until page splitting exists.
    let best = largest_group(&grown, cols, rows)?;

    // Measure the box from the real ink inside that group, not the grown mask,
    // so the padding stays honest.
    let (mut x0, mut y0, mut x1, mut y1) = (cols, rows, 0u32, 0u32);
    let mut count = 0u32;
    for cy in 0..rows {
        for cx in 0..cols {
            if cell(&best, cx, cy, cols) && cell(&ink, cx, cy, cols) {
                x0 = x0.min(cx);
                y0 = y0.min(cy);
                x1 = x1.max(cx);
                y1 = y1.max(cy);
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        return None;
    }

    // Back to the caller's pixels, with a margin.
    let inv = 1.0 / scale;
    let pad = (w.max(h) as f32 * MARGIN).round();
    let fx0 = (x0.saturating_mul(CELL) as f32 * inv - pad).max(0.0);
    let fy0 = (y0.saturating_mul(CELL) as f32 * inv - pad).max(0.0);
    let fx1 = (x1.saturating_add(1).saturating_mul(CELL) as f32 * inv + pad).min(w as f32);
    let fy1 = (y1.saturating_add(1).saturating_mul(CELL) as f32 * inv + pad).min(h as f32);

    let (bw, bh) = ((fx1 - fx0) as u32, (fy1 - fy0) as u32);
    if bw == 0 || bh == 0 {
        return None;
    }

    let share = (bw as f32 * bh as f32) / (w as f32 * h as f32);
    if !(MIN_AREA..=MAX_AREA).contains(&share) {
        return None;
    }
    Some((fx0 as u32, fy0 as u32, bw, bh))
}

/// Flood fill, four-connected, returning the largest group as a mask.
fn largest_group(mask: &[bool], cols: u32, rows: u32) -> Option<Vec<bool>> {
    // A zero-wide grid has no groups, and `i % cols` below would divide by it.
    // The caller already refuses fewer than four columns; this is so the
    // function cannot be made to panic by a later one that does not.
    if cols == 0 || rows == 0 {
        return None;
    }
    let mut seen = vec![false; mask.len()];
    let mut best: Option<(usize, Vec<bool>)> = None;
    let mut stack: Vec<u32> = Vec::new();

    for start in 0..mask.len() {
        let unvisited_ink =
            !seen.get(start).copied().unwrap_or(true) && mask.get(start).copied().unwrap_or(false);
        if !unvisited_ink {
            continue;
        }
        let Ok(start_cell) = u32::try_from(start) else {
            continue;
        };
        let mut group = vec![false; mask.len()];
        let mut size = 0usize;
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start_cell);

        while let Some(i) = stack.pop() {
            if let Some(g) = group.get_mut(i as usize) {
                *g = true;
            }
            size = size.saturating_add(1);
            // `cols` is non-zero, so neither of these can divide by zero; the
            // checked forms say so without a reader having to go and look.
            let (Some(x), Some(y)) = (i.checked_rem(cols), i.checked_div(cols)) else {
                continue;
            };
            let push = |j: u32, seen: &mut Vec<bool>, stack: &mut Vec<u32>| {
                let at = j as usize;
                if seen.get(at).copied().unwrap_or(true) || !mask.get(at).copied().unwrap_or(false) {
                    return;
                }
                if let Some(s) = seen.get_mut(at) {
                    *s = true;
                }
                stack.push(j);
            };
            if let (true, Some(j)) = (x > 0, i.checked_sub(1)) {
                push(j, &mut seen, &mut stack);
            }
            if let (true, Some(j)) = (x.saturating_add(1) < cols, i.checked_add(1)) {
                push(j, &mut seen, &mut stack);
            }
            if let (true, Some(j)) = (y > 0, i.checked_sub(cols)) {
                push(j, &mut seen, &mut stack);
            }
            if let (true, Some(j)) = (y.saturating_add(1) < rows, i.checked_add(cols)) {
                push(j, &mut seen, &mut stack);
            }
        }
        if best.as_ref().is_none_or(|(n, _)| size > *n) {
            best = Some((size, group));
        }
    }
    best.map(|(_, g)| g)
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

    /// Paint a block that looks like print: dark bands the height of a line
    /// of text, separated by paper. Single-pixel checkerboard would be wrong —
    /// it averages away the moment the image is scaled down, and real text
    /// does not.
    fn textured(img: &mut GrayImage, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            let on_a_line = (y - y0) % 24 < 10;
            for x in x0..x1 {
                let v = if on_a_line && (x - x0) % 14 < 9 { 20 } else { 240 };
                img.put_pixel(x, y, image::Luma([v]));
            }
        }
    }

    #[test]
    fn a_document_in_a_large_frame_is_found() {
        let mut img = GrayImage::from_pixel(1200, 1600, image::Luma([245u8]));
        textured(&mut img, 400, 300, 800, 1300);
        let (x, y, w, h) = content_bounds(&img).expect("should find the document");
        // Within the margin we deliberately add.
        assert!(x < 400 && y < 300, "box starts before the ink: {x},{y}");
        assert!(x + w > 800 && y + h > 1300, "box ends after the ink");
        assert!(w < 700 && h < 1300, "box should still be much smaller than the frame");
    }

    #[test]
    fn a_blank_frame_finds_nothing() {
        let img = GrayImage::from_pixel(1200, 1600, image::Luma([245u8]));
        assert_eq!(content_bounds(&img), None);
    }

    /// A shadow is a smooth gradient. Dark is not the same as inky, and a
    /// photograph of a receipt on a table nearly always has one.
    #[test]
    fn a_shadow_is_not_mistaken_for_a_document() {
        let mut img = GrayImage::from_pixel(1200, 1600, image::Luma([245u8]));
        for y in 0..1600u32 {
            for x in 0..600u32 {
                img.put_pixel(x, y, image::Luma([(120 + x / 10) as u8]));
            }
        }
        textured(&mut img, 700, 400, 1000, 1200);
        let (x, _, w, _) = content_bounds(&img).expect("should find the document");
        assert!(x > 600, "the shadow was included, box starts at {x}");
        assert!(w < 500, "box is too wide, {w}");
    }

    /// Two receipts in one frame: take the bigger one rather than a box that
    /// spans both and is mostly table.
    #[test]
    fn the_larger_of_two_documents_wins() {
        let mut img = GrayImage::from_pixel(1600, 1600, image::Luma([245u8]));
        textured(&mut img, 100, 100, 250, 250); // small
        textured(&mut img, 700, 500, 1400, 1400); // large
        let (x, y, _, _) = content_bounds(&img).expect("should find a document");
        assert!(x > 400 && y > 300, "picked the small one: {x},{y}");
    }

    #[test]
    fn a_frame_that_is_all_document_is_left_alone() {
        let mut img = GrayImage::from_pixel(1000, 1000, image::Luma([245u8]));
        textured(&mut img, 5, 5, 995, 995);
        assert_eq!(content_bounds(&img), None, "nothing worth cropping");
    }
}
