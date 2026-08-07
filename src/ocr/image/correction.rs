//! Mapping a point on the straightened page back to the original pixels.
//!
//! Straightening a page and then cropping from the straightened copy resamples
//! every glyph twice: once for the rotation, once for the crop. Two passes of
//! bilinear blur measurably hurts recognition — on a 7 degree tilt it turned
//! `QUICKMART` into `OUICXMART`.
//!
//! So detection runs on the straightened page, where it is easy, but the crops
//! handed to the recogniser are sampled straight from the original image
//! through this transform. One resample, not two.

use super::orient::Orientation;

/// The geometry that was applied to straighten a page.
#[derive(Debug, Clone, Copy)]
pub struct Correction {
    pub orientation: Orientation,
    /// Degrees of skew that were removed.
    pub skew: f32,
    /// Size of the original image, before anything was applied.
    pub original: (u32, u32),
    /// Size after the quarter turn, which is also the straightened size.
    pub turned: (u32, u32),
    /// Where the document was found in the photograph it was cut from.
    ///
    /// [`to_original`](Self::to_original) returns coordinates on the cut-out,
    /// because that is what sampling a crop needs. Add this to get back to the
    /// photograph the user actually took, which is what a "show me where this
    /// number came from" feature needs.
    pub crop_origin: (u32, u32),
}

impl Correction {
    pub fn identity(width: u32, height: u32) -> Self {
        Correction {
            orientation: Orientation::Upright,
            skew: 0.0,
            original: (width, height),
            turned: (width, height),
            crop_origin: (0, 0),
        }
    }

    /// True when nothing was changed, so crops can be taken directly.
    pub fn is_identity(&self) -> bool {
        self.orientation == Orientation::Upright && self.skew.abs() < 0.01
    }

    /// Map a point on the straightened page back to the photograph the user
    /// took, including the crop that found the document inside it.
    pub fn to_frame(&self, x: f32, y: f32) -> (f32, f32) {
        let (ox, oy) = self.to_original(x, y);
        (ox + self.crop_origin.0 as f32, oy + self.crop_origin.1 as f32)
    }

    /// Map a point on the straightened page back to the original image.
    pub fn to_original(&self, x: f32, y: f32) -> (f32, f32) {
        let (tx, ty) = self.undo_skew(x, y);
        self.undo_turn(tx, ty)
    }

    /// The straightening rotation was `rotate(image, -skew)`, which for an
    /// output pixel samples the source at this position. Reusing the same
    /// formula walks us back.
    fn undo_skew(&self, x: f32, y: f32) -> (f32, f32) {
        if self.skew.abs() < 0.01 {
            return (x, y);
        }
        let theta = (-self.skew).to_radians();
        let (sin, cos) = theta.sin_cos();
        let cx = self.turned.0 as f32 / 2.0;
        let cy = self.turned.1 as f32 / 2.0;
        let dx = x - cx;
        let dy = y - cy;
        (cos * dx + sin * dy + cx, -sin * dx + cos * dy + cy)
    }

    fn undo_turn(&self, x: f32, y: f32) -> (f32, f32) {
        let (ow, oh) = (self.original.0 as f32, self.original.1 as f32);
        match self.orientation {
            Orientation::Upright => (x, y),
            // Clockwise put original (x,y) at (h-1-y, x).
            Orientation::Rotated90 => (y, oh - 1.0 - x),
            // Counter-clockwise put original (x,y) at (y, w-1-x).
            Orientation::Rotated270 => (ow - 1.0 - y, x),
            Orientation::UpsideDown => (ow - 1.0 - x, oh - 1.0 - y),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_a_point_to_itself() {
        let c = Correction::identity(100, 50);
        assert!(c.is_identity());
        let (x, y) = c.to_original(30.0, 20.0);
        assert!((x - 30.0).abs() < 0.001 && (y - 20.0).abs() < 0.001);
    }

    #[test]
    fn a_quarter_turn_is_reversed() {
        // A 100x50 original, turned clockwise, becomes 50x100.
        let c = Correction {
            orientation: Orientation::Rotated90,
            skew: 0.0,
            original: (100, 50),
            turned: (50, 100),
            crop_origin: (0, 0),
        };
        // Original top-left (0,0) landed at turned (h-1-0, 0) = (49, 0).
        let (x, y) = c.to_original(49.0, 0.0);
        assert!(x.abs() < 0.001 && y.abs() < 0.001, "got {x},{y}");
    }

    #[test]
    fn skew_is_reversed_to_within_a_pixel() {
        use super::super::deskew::rotate;
        let mut img = super::super::GrayImage::from_pixel(101, 101, image::Luma([255u8]));
        img.put_pixel(70, 30, image::Luma([0u8]));

        let skew = 7.0f32;
        let straightened = rotate(&img, -skew);
        let c = Correction {
            orientation: Orientation::Upright,
            skew,
            original: (101, 101),
            turned: (101, 101),
            crop_origin: (0, 0),
        };

        // Find where the dark pixel ended up, then map it back.
        let mut found = None;
        for y in 0..101u32 {
            for x in 0..101u32 {
                if straightened.get_pixel(x, y).0[0] < 200 {
                    found = Some((x as f32, y as f32));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (sx, sy) = found.expect("the pixel survived the rotation");
        let (ox, oy) = c.to_original(sx, sy);
        assert!(
            (ox - 70.0).abs() < 2.0 && (oy - 30.0).abs() < 2.0,
            "mapped back to {ox},{oy}, expected about 70,30"
        );
    }

    #[test]
    fn upside_down_is_reversed() {
        let c = Correction {
            orientation: Orientation::UpsideDown,
            skew: 0.0,
            original: (100, 50),
            turned: (100, 50),
            crop_origin: (0, 0),
        };
        let (x, y) = c.to_original(0.0, 0.0);
        assert!(
            (x - 99.0).abs() < 0.001 && (y - 49.0).abs() < 0.001,
            "got {x},{y}"
        );
    }
}
