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

    /// Recognise several crops.
    ///
    /// **Every element of the answer must be exactly what [`Recognizer::recognize`]
    /// would return for that crop alone.** A backend may group the work however
    /// it likes; it may not let one crop change another's answer. The default
    /// below satisfies that by construction.
    ///
    /// This used to read "a real backend should override this, because
    /// batching is where the speed comes from", and that sentence cost a
    /// merchant name. The ONNX backend stacks crops into one tensor and pads
    /// each one out to the widest in its group; the recognition network mixes
    /// across the whole sequence, so the padding is not inert. Measured on a
    /// rendered eighteen-row statement, 82 crops, PP-OCRv4:
    ///
    /// | Crops per call | Inter-word spaces read | Time |
    /// |---|---|---|
    /// | 1 | **38 of 38** | 1,180 ms |
    /// | 8 | 37 of 38 | 1,298 ms |
    /// | 16 | 33 of 38 | 1,506 ms |
    /// | 32 | 24 of 38 | 1,838 ms |
    ///
    /// It was slower *and* wrong, and it was wrong in the quietest way
    /// available: mean confidence went **up**, 0.9904 to 0.9947, because the
    /// character it stopped emitting was the only unsure one. A backend that
    /// wants to batch must first show that it does not change the text.
    fn recognize_batch(&self, crops: &[GrayImage]) -> Result<Vec<(String, f32)>> {
        crops.iter().map(|c| self.recognize(c)).collect()
    }
}
