use std::path::PathBuf;

/// Every way anyscan can fail.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported input: {0}")]
    Unsupported(String),

    #[error("image could not be decoded: {0}")]
    Decode(String),

    /// The picture is in a container this build has no decoder for.
    ///
    /// **Not the same fact as [`ScanError::Unsupported`] and not the same fact
    /// as [`ScanError::Decode`].** `Unsupported` means nobody knows what these
    /// bytes are; `Decode` means the file is damaged. This means the file is
    /// fine, we know exactly what it is, and *this build* cannot open it — so
    /// the same photograph read on a phone would succeed.
    ///
    /// The case it was written for is HEIC. Every iPhone photograph is HEIC by
    /// default, and the only pure-Rust option is `libheif`, which is LGPL and
    /// therefore a real question to link statically into a mobile app. iOS and
    /// Android both decode it in the platform, through
    /// [`crate::ocr::image::DecodeImage`]; where nothing does, **a named
    /// refusal is a valid answer** and this is it.
    ///
    /// `why` says what would make it work, because an error that only says no
    /// sends the reader to the wrong problem.
    #[error("{format}: no decoder in this build — {why}")]
    NeedsPlatformDecoder {
        format: &'static str,
        why: &'static str,
    },

    #[error("no text was found in the image")]
    NoText,

    /// The image is larger than we are willing to work on.
    ///
    /// Not a judgement about the picture. A 137 KB PNG declaring 12000 square
    /// decodes to 144 megapixels, and every buffer downstream is sized from
    /// those dimensions.
    #[error("refused: {0}")]
    TooLarge(crate::limits::Exceeded),

    #[error("model {name} is not available locally; run with auto-download or preseed {path}")]
    ModelMissing { name: String, path: PathBuf },

    #[error("model {name} failed its checksum: expected {expected}, found {found}")]
    ModelCorrupt {
        name: String,
        expected: String,
        found: String,
    },

    /// The file loaded is intact, and is not the model the caller declared —
    /// either a different model, or one that cannot do what it was opened in
    /// order to do.
    ///
    /// Distinct from [`ScanError::ModelCorrupt`], which means *the right file,
    /// damaged*. This means *the wrong file, perfectly intact*, and that is
    /// the more dangerous of the two: a damaged model does not load, and a
    /// wrong one reads.
    ///
    /// What its absence cost: this project's documentation claimed Latin
    /// PP-OCRv5 recognition from the day it was written, while every machine
    /// that ever ran it held the **Chinese** PP-OCRv4 recogniser and its
    /// dictionary. A Chinese PaddleOCR dictionary carries the whole ASCII
    /// range, so nothing crashed and nothing came back empty — it returned
    /// plausible Latin text with no way at all to spell `€`, for months, and
    /// every published OCR figure was measured through it. A wrong model is
    /// worse than a missing one, because the missing one stops the run and the
    /// wrong one writes a number into a document.
    ///
    /// `check` names which guarantee broke, so the message says what was
    /// wanted and what was actually there rather than only that something was.
    /// `expected` and `found` are both required for the same reason: a refusal
    /// that does not name the file it actually found sends the reader to the
    /// wrong machine.
    #[error("model {name}: {check} — expected {expected}, found {found}")]
    ModelNotAsDeclared {
        name: String,
        check: &'static str,
        expected: String,
        found: String,
    },

    #[error("inference failed: {0}")]
    Inference(String),
}

pub type Result<T> = std::result::Result<T, ScanError>;
