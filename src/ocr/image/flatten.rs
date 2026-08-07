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

use super::GrayImage;

/// Blur radius as a fraction of the long side. It must be far larger than a
/// stroke of text — otherwise the text becomes its own background and vanishes
/// — and smaller than the lighting changes we are trying to remove.
const RADIUS_FRACTION: f32 = 0.04;
/// Never go below this, or fine print on a small image erases itself.
const MIN_RADIUS: u32 = 8;

/// Even out the lighting across a page.
pub fn flatten(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    if w < 8 || h < 8 {
        return img.clone();
    }
    let radius = ((w.max(h) as f32 * RADIUS_FRACTION) as u32).max(MIN_RADIUS);

    // Summed-area table, so any box mean is four lookups.
    let mut sum = vec![0u64; ((w + 1) * (h + 1)) as usize];
    for y in 0..h {
        let mut row = 0u64;
        for x in 0..w {
            row += img.get_pixel(x, y).0[0] as u64;
            sum[((y + 1) * (w + 1) + x + 1) as usize] = sum[(y * (w + 1) + x + 1) as usize] + row;
        }
    }
    let mean = |x0: u32, y0: u32, x1: u32, y1: u32| -> f32 {
        let a = sum[(y0 * (w + 1) + x0) as usize];
        let b = sum[(y0 * (w + 1) + x1) as usize];
        let c = sum[(y1 * (w + 1) + x0) as usize];
        let d = sum[(y1 * (w + 1) + x1) as usize];
        let n = ((x1 - x0) * (y1 - y0)) as f32;
        (d + a - b - c) as f32 / n.max(1.0)
    };

    let mut out = GrayImage::new(w, h);
    for y in 0..h {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(w);
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
