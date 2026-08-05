//! Coarse page orientation: is the text upright, or is the page on its side?
//!
//! Upright text produces a spiky horizontal projection and a flat vertical one.
//! Comparing the two tells us whether to rotate by 90 degrees. Distinguishing
//! upright from upside down needs a model, so `Orientation::Upright` and
//! `Orientation::UpsideDown` are only separated once a classifier is wired in.

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

/// Decide between upright and sideways using projection spikiness.
pub fn detect(img: &GrayImage) -> Orientation {
    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return Orientation::Upright;
    }
    let mask = ink_mask(img);

    let horizontal = axis_variance(&mask, w, h, true);
    let vertical = axis_variance(&mask, w, h, false);

    if vertical > horizontal * 1.3 {
        Orientation::Rotated90
    } else {
        Orientation::Upright
    }
}

fn axis_variance(mask: &[bool], w: u32, h: u32, by_row: bool) -> f64 {
    let n = if by_row { h } else { w } as usize;
    let mut counts = vec![0u32; n];
    for y in 0..h {
        let row = (y as usize) * (w as usize);
        for x in 0..w {
            if mask[row + x as usize] {
                counts[if by_row { y as usize } else { x as usize }] += 1;
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
    for y in 0..h {
        for x in 0..w {
            let p = *img.get_pixel(x, y);
            if clockwise {
                out.put_pixel(h - 1 - y, x, p);
            } else {
                out.put_pixel(y, w - 1 - x, p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
