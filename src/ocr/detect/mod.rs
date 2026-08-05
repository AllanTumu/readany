//! Text detection: find the quadrilaterals that contain text.

use crate::ocr::error::Result;
use crate::ocr::image::GrayImage;
use crate::ocr::types::Quad;

/// A detection backend. DBNet plugs in here.
pub trait Detector {
    fn detect(&self, image: &GrayImage) -> Result<Vec<Quad>>;
}
