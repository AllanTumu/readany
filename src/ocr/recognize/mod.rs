//! Text recognition: turn a cropped line image into characters.

pub mod ctc;

use crate::ocr::error::Result;
use crate::ocr::image::GrayImage;

/// A recognition backend. The neural model plugs in here.
///
/// Implementations are expected to accept a crop whose height has been
/// normalised (48 px for PP-OCR models) and return the decoded string with a
/// confidence between 0.0 and 1.0.
pub trait Recognizer {
    fn recognize(&self, crop: &GrayImage) -> Result<(String, f32)>;

    /// Recognise a batch. The default runs them one at a time; a real backend
    /// should override this, because batching is where the speed comes from.
    fn recognize_batch(&self, crops: &[GrayImage]) -> Result<Vec<(String, f32)>> {
        crops.iter().map(|c| self.recognize(c)).collect()
    }
}
